#!/usr/bin/env python3
"""Run a copied CLI distribution with no developer PATH or component overrides."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--runtime', required=True)
    parser.add_argument('--sample', required=True)
    parser.add_argument('--report')
    args = parser.parse_args()
    source = Path(args.runtime).resolve()
    sample = str(Path(args.sample).resolve())
    with tempfile.TemporaryDirectory(prefix='framelean-package-') as tmp:
        root = Path(tmp)
        runtime = root/'copied-runtime'
        shutil.copytree(source, runtime, symlinks=True)
        output = root/'output'; output.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.startswith(('FRAMELEAN_', 'DYLD_'))}
        env['FRAMELEAN_RUNTIME_DIR'] = str(root/'connection')
        env['FRAMELEAN_DATA_DIR'] = str(root/'data')
        env['PATH'] = os.pathsep.join([str(Path(os.environ['SystemRoot'])/'System32'), os.environ['SystemRoot']]) if os.name == 'nt' else '/usr/bin:/bin:/usr/sbin:/sbin'
        suffix = '.exe' if os.name == 'nt' else ''
        cli = str(runtime/('framelean'+suffix))
        try:
            for command in (['compress', '--preset', 'quality'], ['convert', '--format', 'mkv']):
                r = subprocess.run([cli, '--json', *command, sample, '--output', str(output), '--wait'],
                                   env=env, cwd=root, capture_output=True, text=True, timeout=120)
                if r.returncode:
                    # Keep diagnostics private; the committed report contains only assertions.
                    raise AssertionError(f'Independent bundle failed ({r.returncode}): {r.stdout} {r.stderr}')
                task = json.loads(r.stdout)['tasks'][0]
                assert task['latest_attempt']['status'] == 'succeeded'
                assert task['task']['plan_json']['schema_version'] == 2
        finally:
            stopped = subprocess.run([cli, '--json', 'worker', 'stop'], env=env, cwd=root, capture_output=True, timeout=30)
            endpoint = root/'connection/endpoint.json'
            if stopped.returncode and endpoint.exists():
                os.kill(json.loads(endpoint.read_text())['pid'], signal.SIGTERM)
        lib = runtime/('framelean_fll.dll' if os.name == 'nt' else 'libframelean_fll.dylib')
        dependency = next(runtime.glob('avutil-*.dll' if os.name == 'nt' else 'libavutil.*.dylib'))
        dependency.rename(dependency.with_suffix('.removed'))
        r = subprocess.run([str(runtime/('framelean-engine'+suffix)), 'probe', str(lib)],
                           env=env, cwd=root, capture_output=True, text=True, timeout=40)
        assert r.returncode and json.loads(r.stdout)['error']['code'] == 'component_unavailable'
    report = {'independent_copy': True, 'minimal_system_path': True, 'no_component_overrides': True,
              'compression_succeeded': True, 'conversion_succeeded': True,
              'missing_bundled_dependency_diagnosed': True}
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
