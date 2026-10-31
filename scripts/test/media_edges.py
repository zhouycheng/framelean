#!/usr/bin/env python3
"""Real media and isolated macOS storage failures; never fill the host disk."""
import argparse
import contextlib
import errno
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from v4_integration import APIError, Worker, ffmpeg, frame_timestamps, wait


@contextlib.contextmanager
def small_volume(root):
    image, mount = root/'storage.sparseimage', root/'volume'
    subprocess.run(['hdiutil', 'create', '-quiet', '-size', '64m', '-fs', 'HFS+',
                    '-type', 'SPARSE', '-volname', 'FrameLeanAcceptance', str(image)], check=True)
    mount.mkdir()
    subprocess.run(['hdiutil', 'attach', '-quiet', '-nobrowse', '-mountpoint', str(mount), str(image)], check=True)
    try:
        yield mount
    finally:
        subprocess.run(['hdiutil', 'detach', '-quiet', str(mount)], check=True)


def fill(volume):
    assert shutil.disk_usage(volume).total < 100 * 1024 * 1024, 'Only fill the dedicated test image'
    path = volume/'filler'
    with path.open('wb', buffering=0) as file:
        for size in (1024*1024, 4096):
            while True:
                try:
                    file.write(b'\x5a'*size)
                except OSError as e:
                    assert e.errno == errno.ENOSPC, e
                    break
    assert shutil.disk_usage(volume).free < 4096
    return path


def main():
    p = argparse.ArgumentParser()
    for key in ('engine', 'library', 'cli', 'sample'):
        p.add_argument('--'+key, required=True)
    p.add_argument('--report')
    p.add_argument('--operation', choices=['compress', 'convert'], default='compress')
    a = p.parse_args()
    engine, library, cli, sample = [str(Path(getattr(a, k)).resolve()) for k in ('engine', 'library', 'cli', 'sample')]
    checks = []
    def passed(name):
        checks.append(name)
        print('PASS '+name, flush=True)
    with tempfile.TemporaryDirectory(prefix='framelean-edges-') as tmp:
        root = Path(tmp)
        vfr, offset = root/'vfr.mp4', root/'offset.mp4'
        ffmpeg('-i', sample, '-t', '4', '-vf', "select='if(lt(t,2),1,not(mod(n,2)))'",
               '-fps_mode', 'vfr', '-c:v', 'libx264', '-an', vfr)
        ffmpeg('-i', sample, '-itsoffset', '0.3', '-i', sample, '-t', '4',
               '-map', '0:v:0', '-map', '1:a:0', '-c', 'copy', offset)
        w = Worker(root/'timing', engine, library, cli)
        try:
            w.start()
            for path in (vfr, offset):
                aid = w.analyze([path])[0]['id']
                task = w.terminal(w.enqueue(aid, operation=a.operation)['task_ids'][0])
                assert task['latest_attempt']['status'] == 'succeeded', task
                result = Path(task['outputs'][0]['final_path'])
                before, after = frame_timestamps(path), frame_timestamps(result)
                assert len(before) == len(after) and max(abs(x-y) for x, y in zip(before, after)) < .002
                assert task['latest_attempt']['validation_json']['passed']
            passed('VFR frames and nonzero AV offset preserved')
            readonly = root/'readonly'; readonly.mkdir(); readonly.chmod(0o555)
            try:
                try:
                    w.enqueue(aid, operation=a.operation, output_directory=str(readonly))
                    raise AssertionError('Read-only directory was accepted')
                except APIError:
                    assert not list(readonly.iterdir())
            finally:
                readonly.chmod(0o755)
            passed('read-only output rejected without files')
        finally:
            w.stop()
        if sys.platform == 'darwin':
            long = root/'long.mp4'
            ffmpeg('-stream_loop', '2', '-i', sample, '-vf', 'scale=1280:720',
                   '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '18', '-c:a', 'copy', long)
            fingerprint = hashlib.sha256(long.read_bytes()).hexdigest()
            with small_volume(root) as volume:
                w = Worker(root/'output-full', engine, library, cli)
                w.out = volume/'outputs'; w.out.mkdir()
                filler = None
                try:
                    w.start(); aid = w.analyze([long])[0]['id']
                    queued = w.enqueue(aid, operation=a.operation)
                    w.active(queued['attempt_ids'][0])
                    filler = fill(volume)
                    task = w.terminal(queued['task_ids'][0])
                    assert task['latest_attempt']['status'] == 'failed', task
                    assert not task['outputs'], task
                    wait(lambda: not list(w.out.iterdir()))
                    assert hashlib.sha256(long.read_bytes()).hexdigest() == fingerprint
                finally:
                    if filler: filler.unlink()
                    w.stop()
                passed('real ENOSPC during encoding, no publication, owned temporary cleaned')
                w = Worker(root/'database-full', engine, library, cli)
                w.data = volume/'data'; w.env['FRAMELEAN_DATA_DIR'] = str(w.data)
                filler = None
                try:
                    w.start(); w.call('queue/pause', {})
                    existing = w.call('analyze', {'inputs': [sample]})['analysis_ids']
                    count = len(w.call('analyses')['items'])
                    filler = fill(volume)
                    try:
                        w.call('analyze', {'inputs': [sample]*100})
                        raise AssertionError('Full database volume accepted transaction')
                    except APIError as error:
                        assert error.value['error']['code'] == 'database_error', error
                    assert w.call('health')['lifecycle'] == 'degraded'
                    assert len(w.call('analyses')['items']) == count
                    filler.unlink(); filler = None
                    w.stop(); w.start()
                    assert w.call('snapshot')['paused']
                    assert w.call('analyses/'+existing[0])['status'] == 'queued'
                finally:
                    if filler: filler.unlink()
                    w.stop()
                passed('real SQLite volume full rolls back, degrades, retains queued work on restart')
        report = {'suite': 'media-edges', 'operation': a.operation, 'platform': sys.platform, 'checks': checks, 'passed': len(checks)}
        if a.report:
            Path(a.report).write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
