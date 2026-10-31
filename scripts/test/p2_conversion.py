#!/usr/bin/env python3
"""Real video/container matrix, presets and preserved stream semantics."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time

from v4_integration import Worker, ffmpeg, frame_timestamps, wait


def video_hashes(path):
    raw = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(path), '-map', '0:v:0',
                                   '-fps_mode', 'passthrough', '-f', 'framemd5', '-'])
    return [line.rsplit(b',', 1)[1].strip() for line in raw.splitlines() if not line.startswith(b'#')]


def main():
    p = argparse.ArgumentParser()
    for key in ('engine', 'library', 'cli', 'sample'):
        p.add_argument('--'+key, required=True)
    p.add_argument('--report')
    a = p.parse_args()
    engine, library, cli, sample = [str(Path(getattr(a, key)).resolve()) for key in ('engine', 'library', 'cli', 'sample')]
    checks, measurements = [], []
    def passed(name):
        checks.append(name); print('PASS '+name, flush=True)
    with tempfile.TemporaryDirectory(prefix='framelean-conversion-') as tmp:
        root = Path(tmp)
        fixtures = []
        for codec in ('h264', 'hevc'):
            base = root/(codec+'.mp4')
            encoder = ['-c:v', 'copy'] if codec == 'h264' else ['-c:v', 'libx265', '-pix_fmt', 'yuv420p', '-tag:v', 'hvc1', '-x265-params', 'pools=1:frame-threads=1']
            ffmpeg('-i', sample, '-t', '3', *encoder, '-c:a', 'copy', '-metadata:s:a:0', 'language=eng', base)
            for container in ('mp4', 'mov', 'mkv'):
                for audio in (True, False):
                    path = root/f'{codec}-{container}-{audio}.{container}'
                    ffmpeg('-i', base, '-map', '0:v:0', *(['-map', '0:a:0'] if audio else ['-an']), '-c', 'copy', '-avoid_negative_ts', 'disabled', path)
                    fixtures.append((path, codec, audio))
        w = Worker(root/'worker', engine, library, cli)
        try:
            w.start()
            analyses = w.analyze([p for p, _, _ in fixtures])
            assert all(a['status'] == 'succeeded' for a in analyses), analyses
            for analysis, (path, codec, audio) in zip(analyses, fixtures):
                aid = analysis['id']
                for operation in ('compress', 'convert'):
                    options = w.cli('options', aid, '--operation', operation)
                    expected = 6 if operation == 'compress' else 2
                    assert len(options['available_choices']) == expected, (path.name, options)
                    for container in ('mp4', 'mkv'):
                        w.case_output(f'{path.stem}-{operation}-{container}')
                        started = time.monotonic()
                        done = w.cli(operation, '--analysis-id', aid, '--format', container, '--output', w.out, '--wait')['tasks'][0]
                        assert done['latest_attempt']['status'] == 'succeeded', done
                        plan = done['task']['plan_json']
                        mode = 'remux' if operation == 'convert' and codec == 'h264' else 'transcode'
                        assert plan['parameters']['mode'] == mode, plan
                        output = Path(done['outputs'][0]['final_path'])
                        assert output.suffix == '.'+container and ('-converted' if operation == 'convert' else '-compressed') in output.name
                        before, after = frame_timestamps(path), frame_timestamps(output)
                        assert len(before) == len(after) and max(abs(x-y) for x, y in zip(before, after)) <= .002, (path.name, operation, container, before[-3:], after[-3:])
                        ffmpeg('-i', output, '-f', 'null', '-')
                        if mode == 'remux':
                            assert video_hashes(path) == video_hashes(output), (path.name, 'decoded content changed')
                        assert done['latest_attempt']['validation_json']['passed']
                        wait(lambda: w.task(done['task']['id'])['latest_attempt']['cleanup_state'] == 'done')
                        assert not list(w.out.glob('.framelean-*'))
                        measurements.append({'input': path.name, 'operation': operation, 'output': container, 'mode': mode, 'seconds': round(time.monotonic()-started, 3), 'source_bytes': path.stat().st_size, 'output_bytes': output.stat().st_size, 'frames': len(after)})
                passed('matrix '+path.name)
            aid = analyses[0]['id']
            w.case_output('presets')
            for preset, crf in (('quality', 18), ('balanced', 23), ('small', 28), ('h264_crf23', 23)):
                started = time.monotonic()
                done = w.cli('compress', '--analysis-id', aid, '--preset', preset, '--output', w.out, '--wait')['tasks'][0]
                elapsed = time.monotonic()-started
                plan = done['task']['plan_json']
                assert plan['parameters']['video']['crf'] == crf
                assert plan['selection']['preset'] == ('balanced' if preset == 'h264_crf23' else preset)
                output = Path(done['outputs'][0]['final_path'])
                assert f'crf={crf}.0'.encode() in output.read_bytes()
                score = subprocess.run(['ffmpeg', '-v', 'info', '-i', str(fixtures[0][0]), '-i', str(output),
                                        '-filter_complex', '[0:v][1:v]ssim', '-an', '-f', 'null', '-'],
                                       capture_output=True, text=True, check=True)
                ssim = float(re.findall(r'All:([0-9.]+)', score.stderr)[-1])
                measurements.append({'preset': preset, 'crf': crf, 'seconds': round(elapsed, 3),
                                     'output_bytes': output.stat().st_size, 'ssim': ssim})
            assert len(list(w.out.glob('*.mp4'))) == 4
            passed('three presets, legacy alias and collision-safe output')
            chapter_meta = root/'chapters.txt'
            chapter_meta.write_text(';FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=part\n')
            chapter = root/'chapters.mkv'
            ffmpeg('-i', fixtures[0][0], '-i', chapter_meta, '-map_metadata', '1', '-map_chapters', '1', '-c', 'copy', chapter)
            ca = w.analyze([chapter])[0]
            assert ca['media_json']['chapter_count'] == 1
            for operation in ('compress', 'convert'):
                options = w.cli('options', ca['id'], '--operation', operation)
                assert options['available_choices'] == [], options
                count = len(w.call('tasks')['items'])
                failure = w.cli(operation, '--analysis-id', aid, '--analysis-id', ca['id'], '--format', 'mp4', '--output', w.out, expected=2)
                assert failure['error']['code'] == 'no_viable_plan'
                assert len(w.call('tasks')['items']) == count
            passed('chapters identified, incompatible batch rejected before enqueue')
            subtitle = root/'sample.srt'; subtitle.write_text('1\n00:00:00,000 --> 00:00:01,000\nHello\n')
            with_subtitle, attached, tenbit = [root/(name+'.mkv') for name in ('subtitle', 'attachment', 'tenbit')]
            ffmpeg('-i', fixtures[0][0], '-i', subtitle, '-map', '0', '-map', '1', '-c', 'copy', with_subtitle)
            ffmpeg('-i', fixtures[0][0], '-c', 'copy', '-attach', subtitle, '-metadata:s:t', 'mimetype=text/plain', attached)
            ffmpeg('-i', fixtures[0][0], '-t', '1', '-c:v', 'libx265', '-pix_fmt', 'yuv420p10le',
                   '-x265-params', 'pools=1:frame-threads=1', '-an', tenbit)
            extra = w.analyze([with_subtitle, attached, tenbit])
            assert 'subtitle' in [t['kind'] for t in extra[0]['media_json']['tracks']]
            assert 'attachment' in [t['kind'] for t in extra[1]['media_json']['tracks']]
            for analysis in extra:
                for operation in ('compress', 'convert'):
                    assert w.cli('options', analysis['id'], '--operation', operation)['available_choices'] == []
            passed('subtitle, attachment and HEVC 10-bit facts exclude unsupported choices')
            w.case_output('mixed-conversion')
            mixed = w.cli('convert', '--analysis-id', aid, '--analysis-id', analyses[6]['id'], '--format', 'mkv', '--output', w.out, '--wait')
            assert [t['latest_attempt']['status'] for t in mixed['tasks']] == ['succeeded', 'succeeded']
            assert [t['task']['plan_json']['parameters']['mode'] for t in mixed['tasks']] == ['remux', 'transcode']
            passed('mixed H264 and HEVC batch shares selection with distinct execution chains')
            language = root/'extended-language.mkv'
            ffmpeg('-i', fixtures[0][0], '-c', 'copy', '-metadata:s:a:0', 'language=zh-Hans',
                   '-metadata:s:v:0', 'language=zh-Hans', language)
            la = w.analyze([language])[0]
            for operation, count in [('compress', 3), ('convert', 1)]:
                options = w.cli('options', la['id'], '--operation', operation)
                assert len(options['available_choices']) == count
                assert all(c['container'] == 'mkv' for c in options['available_choices'])
                assert options['items'][0]['result']['diagnostics'][0]['code'] == 'mp4_language_unsupported'
                before = len(w.call('tasks')['items'])
                failure = w.cli(operation, '--analysis-id', la['id'], '--format', 'mp4', '--output', w.out, expected=2)
                assert failure['error']['code'] == 'no_viable_plan'
                assert len(w.call('tasks')['items']) == before
                done = w.cli(operation, '--analysis-id', la['id'], '--format', 'mkv', '--output', w.out, '--wait')['tasks'][0]
                assert done['latest_attempt']['status'] == 'succeeded' and done['latest_attempt']['validation_json']['passed']
            passed('extended language hides MP4 before enqueue and remains preserved in MKV')
            invalid = w.cli('convert', '--analysis-id', aid, '--output', w.out, expected=2)
            assert invalid['error']['code'] == 'invalid_arguments'
            w.cli('compress', '--analysis-id', aid, '--preset', 'unknown', '--output', w.out, expected=2)
            passed('CLI requires conversion format and rejects unknown preset')
        finally:
            w.stop()
    report = {'suite': 'p2-conversion', 'fixtures': len(fixtures), 'checks': checks, 'passed': len(checks), 'measurements': measurements}
    if a.report: Path(a.report).write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
