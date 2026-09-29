#!/usr/bin/env python3
"""Bounded external diagnostics for the unchanged historical Surreal binary.

Not a latency measurement: samples the isolated fixture while the original
append workload runs. Does not change storage statements or the saved binary.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request


def sql(statement):
    request = urllib.request.Request('http://127.0.0.1:25475/sql', data=statement.encode(), headers={
        'Authorization': 'Basic ' + base64.b64encode(b'root:c2-local-only').decode(),
        'Surreal-NS': 'c2', 'Surreal-DB': 'c2', 'Accept': 'application/json',
    })
    with urllib.request.urlopen(request, timeout=3) as response:
        rows = json.load(response)
    if any(row['status'] != 'OK' for row in rows):
        raise RuntimeError('diagnostic database query failed')
    return rows[0]['result']


def capture(command):
    result = subprocess.run(command, text=True, capture_output=True, timeout=5)
    return {'returncode': result.returncode, 'stdout': result.stdout}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=300)
    parser.add_argument('--wire', action='store_true')
    parser.add_argument('--chunked', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    container = subprocess.check_output(['docker', 'compose', '-p', 'sockudo-c2-append-storage', '-f',
        'audits/performance-2026-09-05/c2/compose.yaml', 'ps', '-q', 'surrealdb'], text=True).strip()
    if not container or '\n' in container:
        raise RuntimeError('expected exactly one isolated Surreal fixture')
    sql('DEFINE NAMESPACE IF NOT EXISTS c2; DEFINE DATABASE IF NOT EXISTS c2;')
    before_tables = set(sql('INFO FOR DB;')['tables'])
    binary = args.binary.resolve()
    env = dict(os.environ)
    for name in ['C2_CHUNKED', 'C2_WIRE_BYTES', 'C2_WIRE_PHASES', 'C2_WIRE_BUDGETS', 'C2_WRITE_METRICS']:
        env.pop(name, None)
    env.update(C2_BACKENDS='surrealdb', C2_APPENDS='512', C2_FRAGMENTS='256', C2_REPS='1',
               C2_VOLUME_ONLY='1', C2_WRITE_METRICS='1', SSL_CERT_FILE='/etc/ssl/cert.pem', AWS_EC2_METADATA_DISABLED='true')
    if args.chunked:
        env['C2_CHUNKED'] = '1'
    if args.wire:
        env.pop('C2_WRITE_METRICS', None)
        env['C2_WIRE_BYTES'] = '1'
    command = [str(binary), 'history::c2_bench::c2_durable_append_storage', '--ignored', '--exact', '--nocapture', '--test-threads=1']
    started = time.monotonic()
    manifest = {'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'timeout_seconds': args.timeout, 'container': container, 'wire_proxy': args.wire, 'chunked': args.chunked,
                'container_image': capture(['docker', 'inspect', '--format', '{{.Config.Image}}', container]),
                'scope': 'external diagnostic sampling; no latency claim; baseline storage code unchanged'}
    stream_tables = []
    timed_out = False
    with (args.output / 'workload.log').open('w') as log, (args.output / 'samples.jsonl').open('w') as samples:
        child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            while child.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed >= args.timeout:
                    timed_out = True
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
                    break
                sample = {'elapsed_seconds': elapsed}
                try:
                    sample['container_stats'] = capture(['docker', 'stats', '--no-stream', '--format', '{{json .}}', container])
                    sample['client_process'] = capture(['ps', '-p', str(child.pid), '-o', 'pid=,%cpu=,rss=,etime=,state='])
                    if not stream_tables:
                        stream_tables = sorted(name for name in set(sql('INFO FOR DB;')['tables']) - before_tables
                                               if name.startswith('c2b') and name.endswith('_version_streams'))
                    sample['stream_progress'] = {name: sql(f'SELECT next_delivery_serial, newest_delivery_serial FROM {name};')
                                                 for name in stream_tables}
                except Exception as error:
                    sample['sampler_error_type'] = type(error).__name__
                samples.write(json.dumps(sample) + '\n')
                samples.flush()
                time.sleep(min(3, max(0, args.timeout - (time.monotonic() - started))))
        finally:
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=5)
        manifest.update(returncode=child.returncode, timed_out=timed_out,
                        elapsed_seconds=time.monotonic() - started, stream_tables=stream_tables,
                        container_state=capture(['docker', 'inspect', '--format', '{{json .State}}', container]))
    (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({'timed_out': timed_out, 'returncode': child.returncode, 'output': str(args.output)}))


if __name__ == '__main__':
    main()
