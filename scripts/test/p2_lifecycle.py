#!/usr/bin/env python3
"""Conversion cancellation, isolated crashes, publication and persisted-plan upgrade."""
import argparse
import http.client
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time

from v4_integration import Worker, ffmpeg, rejects, wait


def main():
    p = argparse.ArgumentParser()
    for key in ('engine', 'library', 'cli', 'sample'):
        p.add_argument('--'+key, required=True)
    p.add_argument('--report')
    a = p.parse_args()
    engine, library, cli, sample = [str(Path(getattr(a, key)).resolve()) for key in ('engine', 'library', 'cli', 'sample')]
    checks = []
    def passed(name):
        checks.append(name); print('PASS '+name, flush=True)
    with tempfile.TemporaryDirectory(prefix='framelean-convert-life-') as tmp:
        root = Path(tmp)
        long = root/'long.mp4'
        ffmpeg('-stream_loop', '3', '-i', sample, '-vf', 'scale=1280:720', '-c:v', 'libx264',
               '-preset', 'ultrafast', '-threads', '2', '-crf', '18', '-c:a', 'copy', long)
        w = Worker(root/'worker', engine, library, cli)
        try:
            w.start()
            aid, long_id = [v['id'] for v in w.analyze([sample, long])]
            for outcome in ('failed', 'canceled'):
                accepted = w.enqueue(long_id, operation='convert', container='mkv')
                active = w.active(accepted['attempt_ids'][0])
                if outcome == 'failed':
                    os.kill(active['host_pid'], getattr(signal, 'SIGKILL', signal.SIGTERM))
                else:
                    if hasattr(signal, 'SIGSTOP'): os.kill(active['host_pid'], signal.SIGSTOP)
                    start = time.monotonic()
                    w.call('cancel', {'attempt_id': accepted['attempt_ids'][0]})
                task = w.terminal(accepted['task_ids'][0])
                assert task['latest_attempt']['status'] == outcome, task
                assert not task['outputs']
                if outcome == 'canceled': assert time.monotonic()-start < 5
            accepted = w.enqueue(long_id, operation='convert', container='mkv')
            w.active(accepted['attempt_ids'][0]); w.kill(); w.start()
            assert w.terminal(accepted['task_ids'][0])['latest_attempt']['status'] == 'interrupted'
            wait(lambda: w.task(accepted['task_ids'][0])['latest_attempt']['cleanup_state'] == 'done')
            w.call('queue/resume', {})
            passed('running conversion Worker abort preserves interrupted state and cleans owned output')
            good = w.terminal(w.enqueue(aid, operation='convert', container='mkv')['task_ids'][0])
            assert good['latest_attempt']['status'] == 'succeeded'
            passed('conversion Host crash, bounded cancel and subsequent task success')
            for point, status in [('validating', 'interrupted'), ('publish_intent', 'interrupted'),
                                  ('publish_link', 'succeeded'), ('terminal_committed', 'succeeded')]:
                w.stop(); w.start(fault=point)
                w.case_output(point)
                accepted = w.enqueue(aid, operation='convert', container='mkv')
                w.call('queue/resume', {})
                wait(lambda: w.proc.poll() is not None)
                w.start()
                task = w.terminal(accepted['task_ids'][0])
                assert task['latest_attempt']['status'] == status, (point, task)
                wait(lambda: w.task(accepted['task_ids'][0])['latest_attempt']['cleanup_state'] == 'done')
                assert len(list(w.out.glob('*.mkv'))) == (1 if status == 'succeeded' else 0), (point, list(w.out.iterdir()))
            passed('conversion abort recovery at validation, publish intent, link and successful commit')
            w.stop(); w.start(fault='cancel_committed')
            accepted = w.enqueue(aid, operation='convert', container='mkv')
            try: w.call('cancel', {'attempt_id': accepted['attempt_ids'][0]})
            except (OSError, http.client.RemoteDisconnected): pass
            wait(lambda: w.proc.poll() is not None)
            w.start()
            assert w.terminal(accepted['task_ids'][0])['latest_attempt']['status'] == 'canceled'
            passed('conversion cancellation intent survives Worker abort')
            w.case_output('observer')
            accepted = w.enqueue(aid, operation='convert', container='mkv')
            observer = subprocess.Popen([cli, '--json', 'watch', accepted['task_ids'][0]], env=w.env,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(.2); observer.terminate(); observer.wait(timeout=5)
            assert w.task(accepted['task_ids'][0])['latest_attempt']['status'] == 'queued'
            w.call('queue/resume', {}); w.cli('watch', accepted['task_ids'][0])
            w.cli('convert', '--analysis-id', aid, '--format', 'mkv', '--output', w.out, '--wait')
            assert len(list(w.out.glob('*.mkv'))) == 2
            passed('conversion observer exit/reconnect and naming collisions')
            changed = root/'changed.mp4'; shutil.copy2(sample, changed)
            ca = w.analyze([changed])[0]['id']; item = w.item(ca, 'convert', 'mkv')
            os.utime(changed, ns=(changed.stat().st_atime_ns, changed.stat().st_mtime_ns+1000000000))
            rejects(lambda: w.call('execute', {'items': [item], 'output_directory': str(w.out)}), 'batch_invalid')
            w.call('queue/pause', {})
            gone = root/'gone'; gone.mkdir()
            accepted = w.enqueue(aid, operation='convert', container='mkv', output_directory=str(gone))
            gone.rmdir(); w.call('queue/resume', {})
            assert w.terminal(accepted['task_ids'][0])['latest_attempt']['status'] == 'failed'
            passed('conversion rejects changed source and missing output directory')
            w.stop(); w.start(fault='publish_link'); w.case_output('legacy-publishing')
            publishing = w.enqueue(aid); w.call('queue/resume', {})
            wait(lambda: w.proc.poll() is not None)
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                legacy = json.loads(db.execute('SELECT plan_json FROM processing_task WHERE id=?', (publishing['task_ids'][0],)).fetchone()[0])
                legacy['schema_version'] = 1
                legacy['parameters'] = {**legacy['parameters']['video'], 'container': 'mp4'}
                del legacy['output']
                db.execute('UPDATE processing_task SET plan_json=? WHERE id=?', (json.dumps(legacy), publishing['task_ids'][0]))
            w.start()
            assert w.terminal(publishing['task_ids'][0])['latest_attempt']['status'] == 'succeeded'
            wait(lambda: w.task(publishing['task_ids'][0])['latest_attempt']['cleanup_state'] == 'done')
            passed('legacy publishing plan reconciles registered output before new-plan checks')
            w.call('queue/pause', {})
            queued = w.enqueue(aid); w.stop()
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                legacy = json.loads(db.execute('SELECT plan_json FROM processing_task WHERE id=?', (queued['task_ids'][0],)).fetchone()[0])
                legacy['schema_version'] = 1
                legacy['parameters'] = {**legacy['parameters']['video'], 'container': 'mp4'}
                del legacy['output']
                db.execute('UPDATE processing_task SET plan_json=? WHERE id=?', (json.dumps(legacy), queued['task_ids'][0]))
            w.start()
            old = w.task(queued['task_ids'][0])
            assert old['latest_attempt']['status'] == 'interrupted' and old['latest_attempt']['error_json']['code'] == 'plan_required'
            rejects(lambda: w.call('retry', {'task_id': queued['task_ids'][0]}), 'plan_required')
            assert w.task(good['task']['id'])['latest_attempt']['status'] == 'succeeded'
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                assert db.execute('PRAGMA user_version').fetchone()[0] == 2
            passed('old queued plan retired, history retained, schema unchanged and retry requires replanning')
        finally:
            w.kill()
    report = {'suite': 'p2-lifecycle', 'checks': checks, 'passed': len(checks)}
    if a.report: Path(a.report).write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
