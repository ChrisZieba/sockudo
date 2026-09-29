#!/usr/bin/env python3
"""Actual master/C2/followup binaries: off rolling compatibility and enabled rollback."""
import os
import pathlib
import subprocess
import uuid

root = pathlib.Path.cwd()
out = pathlib.Path(os.environ.get('C2_MIXED_RESULTS', root / 'audits/performance-2026-09-05/c2-followup/results/mixed'))
out.mkdir(parents=True, exist_ok=True)
binaries = {'master': root/'target/c2/sockudo-durable-baseline-tree',
            'c2':root/'target/c2/sockudo-durable-candidate-final',
            'followup':pathlib.Path(os.environ.get('C2_FOLLOWUP_BINARY', root/'target/c2/sockudo-durable-followup'))}
for source, enabled in [('master', False), ('master', True), ('c2', True)]:
    case = f'{source}-chunked{int(enabled)}'
    prefix = 'c2m' + uuid.uuid4().hex[:12]
    env = dict(os.environ, C2_MIXED_PREFIX=prefix)
    env.pop('C2_CHUNKED', None)
    def run(build, step, suffix='', chunked=False, materialize=False):
        lane_env = dict(env, C2_MIXED_STEP=step)
        if chunked:
            lane_env['C2_CHUNKED'] = '1'
        test = 'history::c2_tests::c2_mixed_version_materialize' if materialize else 'history::c2_bench::c2_mixed_version'
        name = f'{case}-{build}-{step}{suffix}.log'
        result = subprocess.run([str(binaries[build]), test, '--exact', '--ignored', '--nocapture', '--test-threads', '1'], env=lane_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (out/name).write_text(result.stdout)
        print(name, result.returncode, flush=True)
        if result.returncode:
            raise SystemExit(result.returncode)
        return result.stdout
    run(source, 'legacy_write')
    run('followup', 'continue', chunked=enabled)
    for reader in ('master','c2'):
        text = run(reader,'legacy_read','-before')
        if not enabled and (text.count('latest_matches_reference,Some(true)') != 5 or 'error:' in text):
            raise RuntimeError('marker-off rolling read mismatch')
        if enabled and text.count('error:') < 5:
            raise RuntimeError('older decoder did not fail closed')
    if enabled:
        run('followup','materialize', materialize=True)
        for reader in ('master','c2'):
            text = run(reader,'legacy_read','-after')
            if text.count('latest_matches_reference,Some(true)') != 5 or 'error:' in text:
                raise RuntimeError('rollback read mismatch')
    run('followup','cleanup')
