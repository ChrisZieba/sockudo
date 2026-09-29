#!/usr/bin/env python3
"""Measure all three already-built artifacts on one fresh Linux VM, serially.

Only one backend is running at a time. Compiler work is in separate jobs. Each
accepted sample retains the unchanged strict quiet-host gate evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import socket
import subprocess
import sys
import time
import urllib.request

PHASES = ('baseline', 'c2', 'followup')
BACKENDS = {'postgres': 25471, 'mysql': 25472, 'dynamodb': 25473,
            'scylladb': 25474, 'surrealdb': 25475}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_artifacts(artifacts, expected_commit=None):
    manifests = {}
    for phase in PHASES:
        directory = artifacts / phase
        manifest = json.loads((directory / 'manifest.json').read_text())
        if manifest['phase'] != phase:
            raise ValueError('artifact phase mismatch')
        for kind in ['memory', 'durable']:
            path = directory / kind
            if sha(path) != manifest['binaries'][kind]:
                raise ValueError(f'binary hash mismatch: {phase}/{kind}')
            path.chmod(0o755)
        manifests[phase] = manifest
    for field in ['followup_commit', 'rustc', 'cargo', 'cargo_lock_sha256',
                  'memory_profile', 'durable_profile', 'build_jobs', 'cargo_config_sha256', 'build_environment']:
        if len({json.dumps(m[field], sort_keys=True) for m in manifests.values()}) != 1:
            raise ValueError(f'builds do not match: {field}')
    if expected_commit and any(m['followup_commit'] != expected_commit for m in manifests.values()):
        raise ValueError('artifact commit does not match this workflow checkout')
    memory = 'crates/sockudo-core/examples/c2_append_storage.rs'
    if len({m['harnesses'][memory] for m in manifests.values()}) != 1:
        raise ValueError('memory workloads differ')
    durable = 'crates/sockudo-server/src/history/c2_bench.rs'
    if manifests['baseline']['harnesses'][durable] != manifests['c2']['harnesses'][durable]:
        raise ValueError('historical durable workloads differ')
    return manifests


def capture(command):
    return subprocess.check_output(command, text=True)


def container_pids(name):
    # docker top reports host PIDs. Exempt only this fixture's processes, only
    # at the post-workload gate; startup gate always rejects every heavy PID.
    rows = capture(['docker', 'top', name, '-eo', 'pid']).splitlines()[1:]
    pids = [row.strip() for row in rows if row.strip().isdigit()]
    if not pids:
        raise ValueError('fixture host PIDs could not be inspected')
    return ','.join(pids)


def wait_ready(backend, name):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(('127.0.0.1', BACKENDS[backend]), timeout=1):
                pass
            if backend == 'postgres':
                subprocess.run(['docker', 'exec', name, 'pg_isready', '-U', 'c2', '-d', 'c2'],
                               check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            elif backend == 'mysql':
                subprocess.run(['docker', 'exec', name, 'mysqladmin', 'ping', '-pc2-local-only'],
                               check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            elif backend == 'scylladb':
                subprocess.run(['docker', 'exec', name, 'cqlsh', '127.0.0.1', '25474',
                                '-e', 'SELECT release_version FROM system.local'],
                               check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            elif backend == 'surrealdb':
                with urllib.request.urlopen('http://127.0.0.1:25475/health', timeout=2) as response:
                    if response.status != 200:
                        raise OSError('SurrealDB is not healthy')
            elif backend == 'dynamodb':
                request = urllib.request.Request('http://127.0.0.1:25473/', data=b'{}', headers={
                    'Content-Type': 'application/x-amz-json-1.0',
                    'X-Amz-Target': 'DynamoDB_20120810.ListTables',
                    'X-Amz-Date': '20260929T000000Z',
                    'Authorization': 'AWS4-HMAC-SHA256 Credential=c2/20260929/us-east-1/dynamodb/aws4_request, SignedHeaders=host;x-amz-date, Signature=0000000000000000000000000000000000000000000000000000000000000000'})
                with urllib.request.urlopen(request, timeout=2) as response:
                    if 'TableNames' not in json.load(response):
                        raise OSError('DynamoDB is not ready')
            return
        except (OSError, subprocess.CalledProcessError):
            time.sleep(2)
    raise TimeoutError(f'{backend} did not become ready')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('artifacts', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    if platform.system() != 'Linux':
        raise RuntimeError('remote measurement requires the dedicated Linux job')
    artifacts, output = args.artifacts.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2] / 'c2'
    manifests = verify_artifacts(artifacts, os.environ['GITHUB_SHA'])
    environment = {'uname': capture(['uname', '-a']), 'cpu': capture(['lscpu']),
                   'memory': Path('/proc/meminfo').read_text(),
                   'docker': capture(['docker', 'version']),
                   'runner_image': os.environ.get('ImageVersion', ''),
                   'github_run_id': os.environ.get('GITHUB_RUN_ID', ''),
                   'sources': manifests, 'repetitions': 3, 'cases': ['128:16', '2000:256'],
                   'measurement_platform': 'Linux; compare these three builds only, not macOS timings'}
    (output / 'environment.json').write_text(json.dumps(environment, indent=2) + '\n')
    env = dict(os.environ, C2_QUIET_HOST='1', C2_QUIET_ATTEMPTS='60',
               C2_QUIET_WAIT_SECONDS='10', C2_RESULTS_ROOT=str(output),
               C2_CASES='128:16,2000:256')
    for key in ['C2_CHUNKED', 'C2_WRITE_METRICS', 'C2_WIRE_BYTES', 'C2_WIRE_PHASES',
                'C2_WIRE_BUDGETS', 'C2_VOLUME_ONLY', 'C2_WORKLOAD_PIDS', 'C2_APPEND_PROGRESS']:
        env.pop(key, None)
    log_path = output / 'runner.log'
    def measure(kind, phase, rep, backend=None):
        local = dict(env, C2_FIRST_REP=str(rep))
        if phase == 'followup':
            local['C2_CHUNKED'] = '1'
        command = [sys.executable, str(root / f'run_{kind}.py'),
                   str(artifacts / phase / kind), phase, str(rep)]
        if backend:
            service = 'scylla' if backend == 'scylladb' else backend
            local['C2_WORKLOAD_PIDS'] = container_pids(f'sockudo-c2-append-storage-{service}-1')
            command.extend([backend, str(rep), env['C2_CASES']])
        with log_path.open('a') as log:
            subprocess.run(command, env=local, stdout=log, stderr=subprocess.STDOUT, check=True)
    for rep in range(1, 4):
        for phase in PHASES:
            measure('memory', phase, rep)
    compose = ['docker', 'compose', '-p', 'sockudo-c2-append-storage', '-f', str(root / 'compose.yaml'),
               '-f', str(Path(__file__).with_name('compose.pinned.yaml'))]
    # Pull before the first database measurement; never mix network/image work
    # with accepted samples. A fresh job has no unrelated fixture containers.
    subprocess.run([*compose, 'pull', 'postgres', 'mysql', 'dynamodb', 'scylla', 'surrealdb'], check=True)
    for backend in BACKENDS:
        service = 'scylla' if backend == 'scylladb' else backend
        name = f'sockudo-c2-append-storage-{service}-1'
        subprocess.run([*compose, 'up', '-d', '--no-deps', service], check=True)
        try:
            wait_ready(backend, name)
            (output / f'{backend}-container.json').write_text(capture(['docker', 'inspect', name]))
            for rep in range(1, 4):
                for phase in PHASES:
                    measure('durable', phase, rep, backend)
        finally:
            with (output / f'{backend}-container.log').open('w') as log:
                subprocess.run(['docker', 'logs', name], stdout=log, stderr=subprocess.STDOUT)
            subprocess.run([*compose, 'stop', service], check=True)
    subprocess.run([sys.executable, str(root / 'summarize.py')], env=env, check=True)
    subprocess.run([sys.executable, str(Path(__file__).with_name('validate.py')), str(output)], check=True)


if __name__ == '__main__':
    main()
