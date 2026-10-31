#!/usr/bin/env python3
"""Isolated /v4, FLL contract and real publication fault acceptance. CLI tools are test-only."""
import argparse
import hashlib
import json
import os
import pathlib
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def wait(fn, seconds=90):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        value = fn()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError('condition timed out')


def ffmpeg(*args):
    subprocess.run(['ffmpeg', '-v', 'error', '-nostdin', '-y', *map(str, args)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def frame_timestamps(path):
    data = subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_frames',
                                    '-show_entries', 'frame=best_effort_timestamp_time', '-of', 'json', str(path)])
    return [float(frame['best_effort_timestamp_time']) for frame in json.loads(data)['frames']]


class APIError(Exception):
    def __init__(self, value):
        self.value = value
        super().__init__(json.dumps(value, ensure_ascii=False))


class Worker:
    def __init__(self, root, engine, library, cli):
        self.root, self.engine, self.library, self.cli_path = root, engine, library, cli
        self.runtime, self.data, self.out = root/'runtime', root/'data', root/'中文 输出'
        self.out.mkdir(parents=True, exist_ok=True)
        self.env = {**os.environ, 'FRAMELEAN_RUNTIME_DIR': str(self.runtime),
                    'FRAMELEAN_DATA_DIR': str(self.data), 'FRAMELEAN_ENGINE': engine,
                    'FRAMELEAN_FLL': library, 'FRAMELEAN_TEST_MODE': '1'}
        self.proc = None

    def start(self, fault=None, pause=None, ready=True):
        env = {**self.env, 'FRAMELEAN_TEST_FAULT': fault or '', 'FRAMELEAN_TEST_PAUSE': pause or ''}
        self.log = open(self.root/'worker.log', 'ab')
        self.proc = subprocess.Popen([self.engine, 'worker', self.library], env=env,
                                     stdout=self.log, stderr=self.log)
        def discovered():
            assert self.proc.poll() is None, 'Worker exited during startup; inspect private log'
            try:
                self.ep = json.loads((self.runtime/'endpoint.json').read_text())
                health = self.call('health')
                return health if not ready or health['lifecycle'] in ('ready', 'degraded') else None
            except (OSError, ValueError):
                return None
        return wait(discovered, 40)

    def call(self, name, body=None):
        raw = None if body is None else json.dumps({'request_id': str(uuid.uuid4()), **body}).encode()
        req = urllib.request.Request(self.ep['url']+'/v4/'+name, data=raw,
                                     headers={'Authorization': 'Bearer '+self.ep['token'],
                                              'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=40) as response:
                return json.load(response)
        except urllib.error.HTTPError as e:
            raise APIError(json.load(e)) from e

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.call('shutdown', {})
            self.proc.wait(timeout=20)
            assert not (self.runtime/'endpoint.json').exists()
        if hasattr(self, 'log'):
            self.log.close()

    def kill(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=10)
        if hasattr(self, 'log'):
            self.log.close()

    def analyze(self, paths):
        ids = self.call('analyze', {'inputs': list(map(str, paths))})['analysis_ids']
        self.call('queue/resume', {})
        def done():
            values = [self.call('analyses/'+i) for i in ids]
            return values if all(a['status'] not in ('queued', 'running') for a in values) else None
        return wait(done)

    def item(self, aid, operation='compress', container='mp4'):
        result = self.call('plan', {'analysis_ids': [aid],
                                  'intent': {'operation': operation, 'preservation': 'preserve'},
                                  'selection': {'preset': 'auto' if operation == 'convert' else 'h264_crf23', 'container': container}})
        assert 'result' in result['items'][0], result
        plan = result['items'][0]['result']
        return {'analysis_id': aid, 'intent': plan['intent'], 'selection': plan['selection'],
                'expected_plan_digest': plan['plan_digest']}

    def enqueue(self, aid, operation='compress', container='mp4', **extra):
        return self.call('execute', {'items': [self.item(aid, operation, container)], 'output_directory': str(self.out), **extra})

    def task(self, tid):
        return self.call('tasks/'+tid)

    def case_output(self, name):
        self.out = self.root/name
        self.out.mkdir()

    def terminal(self, tid):
        def done():
            t = self.task(tid)
            return t if t['latest_attempt']['status'] not in ('queued', 'running', 'validating', 'publishing') else None
        return wait(done)

    def active(self, eid):
        def ready():
            a = self.call('snapshot')['active']
            return a if a and a['id'] == eid else None
        return wait(ready)

    def cli(self, *args, expected=0):
        r = subprocess.run([self.cli_path, '--json', *map(str, args)], env=self.env,
                           capture_output=True, text=True, timeout=120)
        assert r.returncode == expected, (r.returncode, r.stdout, r.stderr)
        return json.loads(r.stdout)


def rejects(fn, code):
    try:
        fn()
    except APIError as e:
        assert e.value['error']['code'] == code, e.value
        return e.value
    raise AssertionError('invalid request accepted: '+code)


def host(engine, library, operation, schema=3):
    p = subprocess.Popen([engine, 'host', library], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        assert json.loads(p.stdout.readline())['type'] == 'ready'
        p.stdin.write(json.dumps({'schema_version': schema, 'operation_id': 'test', 'operation': operation})+'\n')
        p.stdin.flush()
        while True:
            e = json.loads(p.stdout.readline())
            if e['state'] in ('succeeded', 'failed', 'canceled'):
                p.wait(timeout=5)
                return e
    finally:
        if p.poll() is None:
            p.kill()
        p.wait()


def main():
    parser = argparse.ArgumentParser()
    for arg in ('engine', 'library', 'cli', 'sample'):
        parser.add_argument('--'+arg, required=True)
    parser.add_argument('--report')
    args = parser.parse_args()
    engine, library, cli, sample = [str(pathlib.Path(getattr(args, x)).resolve()) for x in ('engine', 'library', 'cli', 'sample')]
    checks = []
    def passed(name):
        checks.append(name)
        print('PASS '+name, flush=True)
    with tempfile.TemporaryDirectory(prefix='framelean-v4-') as temp:
        root = pathlib.Path(temp)
        source = root/'真实 视频.mp4'
        shutil.copy2(sample, source)
        no_audio, bad, mpeg4, rotate, hdr, multi = [root/(n+'.mp4') for n in ('silent', 'broken', 'mpeg4', 'rotate', 'hdr', 'multi')]
        ffmpeg('-i', source, '-t', '2', '-c:v', 'copy', '-an', no_audio)
        ffmpeg('-i', source, '-t', '1', '-c:v', 'mpeg4', '-an', mpeg4)
        ffmpeg('-display_rotation', '90', '-i', source, '-t', '1', '-c', 'copy', rotate)
        ffmpeg('-i', source, '-t', '1', '-vf', 'setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc', '-c:v', 'libx264', '-an', hdr)
        ffmpeg('-i', source, '-t', '1', '-map', '0:v:0', '-map', '0:a:0', '-map', '0:a:0', '-c', 'copy', multi)
        bad.write_bytes(b'broken input')
        w = Worker(root/'main', engine, library, cli)
        try:
            assert w.start()['lifecycle'] == 'ready'
            req = {'request_id': 'receipt', 'inputs': [str(source), str(bad)]}
            first = w.call('analyze', req)
            assert w.call('analyze', req) == first
            rejects(lambda: w.call('analyze', {**req, 'inputs': [str(source)]}), 'request_conflict')
            values = w.analyze([source, no_audio, mpeg4, rotate, hdr, multi, bad])
            assert [v['status'] for v in values] == ['succeeded']*6+['failed']
            aid = values[0]['id']
            for operation, count in [('compress', 6), ('convert', 2)]:
                options = w.call('options', {'analysis_ids': [v['id'] for v in values[:6]], 'intent': {'operation': operation, 'preservation': 'preserve'}})
                assert options['available_choices'] == []
                assert [len(i['result']['available_choices']) for i in options['items']] == [count, count, 0, 0, 0, 0], options
            rejects(lambda: w.call('options', {'analysis_ids': [aid], 'format': 123}), 'invalid_format')
            passed('mixed media facts, unsupported choices hidden, batch intersection, request deduplication')

            facts = values[0]['media_json']
            assert facts['tracks'][0]['video']['color']['primaries']['state'] == 'unknown'
            unknown = json.loads(json.dumps(facts))
            unknown['tracks'][0]['video']['width'] = {'state': 'unknown', 'value': None, 'reason': 'test missing width'}
            unknown['facts_digest'] = ''
            unknown['facts_digest'] = hashlib.sha256(json.dumps(unknown, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
            unknown_options = host(engine, library, {'kind': 'options', 'media': unknown, 'intent': {'operation': 'compress', 'preservation': 'preserve'}})
            assert unknown_options['result']['available_choices'] == [], unknown_options
            planned = host(engine, library, {'kind': 'plan', 'media': facts, 'intent': {'operation': 'compress', 'preservation': 'preserve'}, 'selection': {'preset': 'h264_crf23', 'container': 'mp4'}})
            assert planned['state'] == 'succeeded', planned
            plan = planned['result']
            assert plan['parameters']['video']['crf'] == 23 and plan['parameters']['video']['preset'] == 'medium'
            assert host(engine, library, {'kind': 'capabilities'}, schema=999)['error']['code'] == 'protocol_mismatch'
            assert host(engine, library, {'kind': 'capabilities', 'unknown': True})['error']['code'] == 'invalid_request'
            altered = json.loads(json.dumps(plan)); altered['parameters']['video']['crf'] = 1
            execution = {'kind': 'execute', 'input': str(source), 'temp': str(root/'unowned.mp4'), 'media': facts, 'plan': altered, 'grant': {'threads': 2}}
            assert host(engine, library, execution)['error']['code'] == 'plan_stale'
            hardlink = root/'source-hardlink.mp4'; os.link(source, hardlink)
            execution.update(plan=plan, temp=str(hardlink))
            before = hashlib.sha256(source.read_bytes()).hexdigest()
            assert host(engine, library, execution)['error']['code'] == 'output_conflict'
            assert hashlib.sha256(source.read_bytes()).hexdigest() == before
            count = len(w.call('tasks')['items'])
            invalid = w.item(aid); invalid['expected_plan_digest'] = 'obsolete'
            rejects(lambda: w.call('execute', {'items': [w.item(aid), invalid], 'output_directory': str(w.out)}), 'batch_invalid')
            assert len(w.call('tasks')['items']) == count
            passed('JSON version and fields, immutable execution plan, source hardlink protection, atomic batch rejection')

            w.call('queue/pause', {})
            accepted = w.enqueue(aid); tid, eid = accepted['task_ids'][0], accepted['attempt_ids'][0]
            w.call('cancel', {'attempt_id': eid})
            retry = w.call('retry', {'task_id': tid})
            rejects(lambda: w.call('retry', {'task_id': tid}), 'task_active')
            w.stop(); w.start()
            assert w.call('snapshot')['paused']
            assert w.call('analyze', req) == first
            assert w.task(tid)['attempt_count'] == 2
            w.call('queue/resume', {})
            done = w.terminal(tid); assert done['latest_attempt']['status'] == 'succeeded', done
            output = pathlib.Path(done['outputs'][0]['final_path'])
            assert all(p in output.read_bytes() for p in (b'crf=23.0', b'subme=7', b'rc_lookahead=40')), 'encoder parameters differ from planned medium CRF 23'
            ffmpeg('-i', output, '-f', 'null', '-')
            source_pts, result_pts = frame_timestamps(source), frame_timestamps(output)
            assert len(source_pts) == len(result_pts), 'decoded frame count changed (including last frame)'
            assert max(abs(x-y) for x, y in zip(source_pts, result_pts)) < .002
            def packets(path):
                r = subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams', 'a:0', '-show_packets', '-show_data_hash', 'sha256', '-show_entries', 'packet=pts_time,data_hash', '-of', 'json', str(path)])
                return json.loads(r)['packets']
            assert packets(source) == packets(output), 'AAC packets or timestamps changed'
            assert done['latest_attempt']['validation_json']['passed']
            assert done['task']['plan_json']['plan_digest'] == plan['plan_digest']
            w.cli('compress', '--analysis-id', aid, '--output', w.out, '--wait')
            assert output.exists() and len(list(w.out.glob('*.mp4'))) == 2
            w.cli('compress', no_audio, '--output', w.out, '--wait')
            passed('real decode, AAC packet/timestamp equality, silent input, Unicode paths, collision and retry history')

            accepted = w.enqueue(aid); tid, eid = accepted['task_ids'][0], accepted['attempt_ids'][0]
            live = w.active(eid)
            os.kill(live['host_pid'], getattr(signal, 'SIGKILL', signal.SIGTERM))
            assert w.terminal(tid)['latest_attempt']['status'] == 'failed'
            accepted = w.enqueue(aid); tid, eid = accepted['task_ids'][0], accepted['attempt_ids'][0]
            active = w.active(eid)
            if hasattr(signal, 'SIGSTOP'): os.kill(active['host_pid'], signal.SIGSTOP)
            start = time.monotonic(); w.call('cancel', {'attempt_id': eid})
            assert w.terminal(tid)['latest_attempt']['status'] == 'canceled'
            assert time.monotonic()-start < 5
            w.cli('watch', tid, expected=5)
            w.call('queue/pause', {})
            queued = w.cli('compress', '--analysis-id', aid, '--output', w.out)
            observer = subprocess.Popen([cli, '--json', 'watch', queued['task_ids'][0]], env=w.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(.2); observer.terminate(); observer.wait(timeout=5)
            assert w.task(queued['task_ids'][0])['latest_attempt']['status'] == 'queued'
            w.call('queue/resume', {}); w.cli('watch', queued['task_ids'][0])
            passed('Host crash isolation, active cancellation bound, CLI observation exit and reconnect')

            accepted = w.enqueue(aid); tid, eid = accepted['task_ids'][0], accepted['attempt_ids'][0]
            w.active(eid); w.kill(); w.start()
            assert w.terminal(tid)['latest_attempt']['status'] == 'interrupted'
            changed = root/'changed.mp4'; shutil.copy2(source, changed)
            changed_id = w.analyze([changed])[0]['id']; item = w.item(changed_id)
            os.utime(changed, ns=(changed.stat().st_atime_ns, changed.stat().st_mtime_ns+1000000000))
            rejects(lambda: w.call('execute', {'items': [item], 'output_directory': str(w.out)}), 'batch_invalid')
            w.call('queue/pause', {}); gone = root/'gone'; gone.mkdir()
            r = w.enqueue(aid, output_directory=str(gone)); gone.rmdir(); w.call('queue/resume', {})
            assert w.terminal(r['task_ids'][0])['latest_attempt']['status'] == 'failed'
            passed('running Worker interruption, source change and output write failure')
            damaged = root/'late-damage.mp4'; shutil.copy2(source, damaged)
            video_packets = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-select_streams', 'v:0', '-show_packets', '-show_entries', 'packet=pos,size', '-of', 'json', str(damaged)]))['packets']
            packet = video_packets[len(video_packets)*3//4]
            with damaged.open('r+b') as file:
                file.seek(int(packet['pos'])); file.write(b'\0'*int(packet['size']))
            a = w.analyze([damaged])[0]
            assert a['status'] == 'succeeded', 'late-damage fixture should remain probeable'
            for operation in ('compress', 'convert'):
                r = w.enqueue(a['id'], operation=operation)
                task = w.terminal(r['task_ids'][0])
                assert task['latest_attempt']['status'] == 'failed' and not task['outputs']
            passed('probeable late corruption fails execution without publishing damaged result')
            w.stop()

            # Real aborts on each durable publication boundary, followed by recovery.
            for point, expected in [('validating', 'interrupted'), ('publish_intent', 'interrupted'), ('publish_link', 'succeeded'), ('terminal_committed', 'succeeded'), ('cancel_committed', 'canceled')]:
                w.case_output('fault-'+point)
                w.start(fault=point)
                r = w.enqueue(aid); tid, eid = r['task_ids'][0], r['attempt_ids'][0]
                w.call('queue/resume', {})
                if point == 'cancel_committed':
                    w.active(eid)
                    try: w.call('cancel', {'attempt_id': eid})
                    except (OSError, __import__('http.client', fromlist=['RemoteDisconnected']).RemoteDisconnected): pass
                w.proc.wait(timeout=60); assert w.proc.returncode != 0
                w.start(); result = w.terminal(tid)
                assert result['latest_attempt']['status'] == expected, (point, result)
                if expected == 'succeeded': ffmpeg('-i', result['outputs'][0]['final_path'], '-f', 'null', '-')
                w.stop()
            passed('actual abort/restart at validation, intent, link, success commit and persisted cancel intent')

            w.case_output('cancel-race')
            w.start(pause='publish_link')
            r = w.enqueue(aid); tid, eid = r['task_ids'][0], r['attempt_ids'][0]; w.call('queue/resume', {})
            marker = w.runtime/'publish_link.ready'; wait(marker.exists)
            assert w.call('cancel', {'attempt_id': eid})['accepted'] is False
            marker.with_suffix('.go').touch()
            assert w.terminal(tid)['latest_attempt']['status'] == 'succeeded'
            w.stop(); marker.unlink(); marker.with_suffix('.go').unlink()
            passed('cancel during publication converges to successful published result')

            w.case_output('offline-volume')
            w.start(fault='publish_link'); r = w.enqueue(aid); tid = r['task_ids'][0]; w.call('queue/resume', {})
            w.proc.wait(timeout=60)
            offline = w.out.with_name('offline'); w.out.rename(offline)
            w.start(); assert w.task(tid)['latest_attempt']['status'] == 'reconciling'
            offline.rename(w.out); w.call('reconcile', {'attempt_id': r['attempt_ids'][0]})
            wait(lambda: w.task(tid)['latest_attempt']['status'] == 'succeeded'); w.stop()
            passed('unavailable output directory remains reconciling and explicit reconciliation succeeds')

            w.case_output('identity-conflict')
            w.start(fault='publish_intent'); r = w.enqueue(aid); tid = r['task_ids'][0]; w.call('queue/resume', {}); w.proc.wait(timeout=60)
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                temp_path, final_path = db.execute('SELECT e.temp_path,o.final_path FROM execution_attempt e JOIN output_artifact o ON o.attempt_id=e.id WHERE e.id=?', r['attempt_ids']).fetchone()
            shutil.copyfile(temp_path, final_path)
            identical = pathlib.Path(final_path).read_bytes()
            w.start(); assert w.task(tid)['latest_attempt']['status'] == 'failed'
            assert pathlib.Path(final_path).read_bytes() == identical
            w.stop()
            passed('identical unrelated file is preserved rather than mistaken for published hardlink')

            w.case_output('cleanup-failure')
            w.start(fault='terminal_committed'); r = w.enqueue(aid); tid = r['task_ids'][0]; w.call('queue/resume', {}); w.proc.wait(timeout=60)
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                temp_path = pathlib.Path(db.execute('SELECT temp_path FROM execution_attempt WHERE id=?', r['attempt_ids']).fetchone()[0])
            temp_path.unlink(); temp_path.write_bytes(b'foreign replacement')
            w.start()
            wait(lambda: w.task(tid)['latest_attempt']['cleanup_state'] == 'error')
            assert w.task(tid)['latest_attempt']['status'] == 'succeeded' and temp_path.read_bytes() == b'foreign replacement'
            w.stop()
            passed('cleanup identity failure preserves file and successful result')

            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                db.executemany("INSERT INTO execution_attempt(id,task_id,attempt_number,queue_order,status) VALUES(?,?,?,?,'failed')",
                               [(str(uuid.uuid4()), tid, 100+i, 10000+i) for i in range(10000)])
            w.start()
            start = time.monotonic(); page = w.call('tasks/'+tid+'?limit=50'); elapsed = time.monotonic()-start
            assert len(page['attempts']) == 50 and page['attempt_count'] == 10001 and elapsed < 2
            req = urllib.request.Request(w.ep['url']+'/v4/events', headers={'Authorization': 'Bearer '+w.ep['token'], 'Last-Event-ID': 'old-session:1'})
            with urllib.request.urlopen(req, timeout=3) as stream:
                assert b'"type":"reset"' in stream.readline()+stream.readline()
                start = time.monotonic()
                for _ in range(10): w.call('snapshot')
                assert time.monotonic()-start < 2
            passed('10000 attempt history bounded pagination and slow event subscription responsiveness')
            w.stop()
            # A large registered publication exercises out-of-transaction recovery hashing.
            with sqlite3.connect(w.data/'framelean-v2.sqlite3') as db:
                oid = db.execute("SELECT attempt_id FROM output_artifact WHERE status='succeeded' LIMIT 1").fetchone()[0]
                large = root/'large-result.mp4'
                with large.open('wb') as file:
                    file.truncate(512*1024*1024)
                digest = hashlib.sha256()
                with large.open('rb') as file:
                    while block := file.read(1024*1024): digest.update(block)
                st = large.stat()
                identity = json.dumps({'file': st.st_ino, 'volume': st.st_dev}) if os.name == 'posix' else None
                db.execute("UPDATE execution_attempt SET status='publishing' WHERE id=?", (oid,))
                db.execute("UPDATE output_artifact SET status='publishing',final_path=?,size_bytes=?,sha256=?,file_identity_json=? WHERE attempt_id=?", (str(large), st.st_size, digest.hexdigest(), identity, oid))
                large_tid = db.execute('SELECT task_id FROM execution_attempt WHERE id=?', (oid,)).fetchone()[0]
            w.start(ready=False)
            latency = []
            while True:
                start = time.monotonic(); state = w.call('snapshot'); latency.append(time.monotonic()-start)
                if state['lifecycle'] == 'ready': break
                time.sleep(.02)
            assert max(latency) < 2, latency
            assert w.task(large_tid)['latest_attempt']['status'] == 'succeeded'
            w.stop()
            passed('512 MiB publication hash recovery keeps snapshot responsive')
            broken = Worker(root/'missing', engine, str(root/'missing.dylib'), cli)
            try:
                health = broken.start(); assert health['lifecycle'] == 'degraded'
                assert health['health']['error']['code'] == 'component_unavailable'
                broken.stop()
            finally: broken.kill()
            passed('missing FLL is diagnosable without terminating gateway')
            if __import__('platform').system() == 'Darwin':
                for abi, version, code in [(99, 2, 'abi_mismatch'), (1, 99, 'protocol_mismatch')]:
                    stub = root/'stub.c'; lib = root/'stub.dylib'
                    stub.write_text('#include <stdlib.h>\n#include <string.h>\nunsigned fll_abi_version(void){return '+str(abi)+';}\nchar *fll_versions(void){return strdup("{\\"json_version\\":'+str(version)+'}");}\nvoid fll_free(char *p){free(p);}\n')
                    subprocess.run(['cc', '-dynamiclib', str(stub), '-o', str(lib)], check=True, capture_output=True)
                    r = subprocess.run([engine, 'probe', str(lib)], capture_output=True, text=True, timeout=5)
                    assert r.returncode != 0 and json.loads(r.stdout)['error']['code'] == code
                passed('FLL ABI and JSON negotiation diagnose old components')
        finally:
            w.kill()
    report = {'suite': 'v4-integration', 'platform': __import__('platform').system(),
              'checks': checks, 'passed': len(checks), 'windows': 'not tested'}
    if args.report: pathlib.Path(args.report).write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
