#!/usr/bin/env python3
"""Run the C2 durable matrix: one process per backend/case/repetition.

Keeps stdout, /usr/bin/time output and the backend container's cgroup CPU
usage delta (database-side CPU) for every process.

usage: run_durable.py <test-binary> <phase> [repetitions] [backend,...] [first-rep] [N:F,...]
"""
import argparse
import os
from quiet_host import resume_existing, run
from durable_validation import validate_durable_output
import pathlib
import subprocess
import sys
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary')
parser.add_argument('phase')
parser.add_argument('repetitions', type=int, nargs='?', default=3)
parser.add_argument('backends', nargs='?', default='postgres,mysql,dynamodb,scylladb,surrealdb')
parser.add_argument('first_rep', type=int, nargs='?', default=1)
parser.add_argument('cases', nargs='?')
parser.add_argument('--resume', action='store_true', help='reuse verified, unchanged accepted results')
args = parser.parse_args()
binary, phase, reps = args.binary, args.phase, args.repetitions
backends = args.backends.split(',')
# DynamoDB items are limited to 400 KB, and the pre-change latest-state item
# holds two copies of a 512 KB aggregate; SurrealDB stores byte vectors as
# number arrays, making that case impractically large for the emulator VM.
SKIP = {("dynamodb", 2000, 256), ("surrealdb", 2000, 256)}
out = pathlib.Path(os.environ.get("C2_RESULTS_ROOT", pathlib.Path(__file__).parent / "results")) / "durable"
out.mkdir(parents=True, exist_ok=True)


def container_cpu_usec(backend):
    service = "scylla" if backend == "scylladb" else backend
    name = f"sockudo-c2-append-storage-{service}-1"
    try:
        result = subprocess.run(["docker", "exec", name, "cat", "/sys/fs/cgroup/cpu.stat"],
                                capture_output=True, text=True)
    except OSError:
        raise RuntimeError(f'database CPU inspection failed: backend={backend} returncode=unavailable') from None
    if result.returncode == 0:
        for line in result.stdout.splitlines():
            fields = line.split()
            if len(fields) == 2 and fields[0] == 'usage_usec' and fields[1].isdigit():
                return int(fields[1])
    raise RuntimeError(f'database CPU inspection failed: backend={backend} returncode={result.returncode}')


def container_cpu_delta(backend, before):
    after = container_cpu_usec(backend)
    if before < 0 or after < before:
        raise RuntimeError(f'database CPU counter reset: backend={backend} returncode=0')
    return f'container_cpu_usec_delta,{after - before}\n'.encode()


first_rep = args.first_rep
cases = None
if args.cases:
    cases = {tuple(int(part) for part in case.split(":")) for case in args.cases.split(",")}
for rep in range(first_rep, reps + 1):
    for backend in backends:
        for appends in (128, 512, 2000):
            for fragment in (16, 64, 256):
                if (backend, appends, fragment) in SKIP:
                    continue
                if cases is not None and (appends, fragment) not in cases:
                    continue
                stem = out / f"{phase}-{backend}-n{appends}-f{fragment}-r{rep}"
                validator = lambda path: validate_durable_output(path, backend, appends, fragment)
                if resume_existing(stem, args.resume, extra_output_suffixes=("dbcpu",), output_validator=validator):
                    print(phase, rep, backend, appends, fragment, "resume accepted", flush=True)
                    continue
                env = {
                    "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
                    "HOME": str(pathlib.Path.home()),
                    "C2_BACKENDS": backend,
                    "C2_APPENDS": str(appends),
                    "C2_FRAGMENTS": str(fragment),
                    "C2_REPS": "1",
                }
                if os.environ.get("C2_WORKLOAD_PIDS"):
                    env["C2_WORKLOAD_PIDS"] = os.environ["C2_WORKLOAD_PIDS"]
                for flag in ("C2_CHUNKED", "C2_WIRE_BYTES", "C2_WRITE_METRICS", "C2_WIRE_BUDGETS", "C2_WIRE_PHASES"):
                    if os.environ.get(flag):
                        env[flag] = "1"
                env["SSL_CERT_FILE"] = "/etc/ssl/cert.pem"
                env["AWS_EC2_METADATA_DISABLED"] = "true"
                command = ["/usr/bin/time", "-l", binary, "c2_durable_append_storage",
                           "--ignored", "--nocapture", "--test-threads", "1"]
                if os.environ.get("C2_QUIET_HOST"):
                    code = run(
                        stem, command, env=env,
                        before_workload=lambda: container_cpu_usec(backend),
                        after_workload=lambda before: {'dbcpu': container_cpu_delta(backend, before)},
                        output_validator=validator,
                    )
                else:
                    attempt_stem = f'{stem}.attempt-{uuid.uuid4().hex}'
                    try:
                        before = container_cpu_usec(backend)
                        with open(f"{attempt_stem}.csv", "w") as stdout, open(f"{attempt_stem}.time", "w") as stderr:
                            code = subprocess.call(command, stdout=stdout, stderr=stderr, env=env)
                        pathlib.Path(f'{attempt_stem}.dbcpu').write_bytes(container_cpu_delta(backend, before))
                        validator(pathlib.Path(f'{attempt_stem}.csv'))
                    except (OSError, ValueError, RuntimeError):
                        pathlib.Path(f'{attempt_stem}.discarded').write_text('discarded: workload measurement or output validation failed\n')
                        raise
                    if code == 0:
                        for suffix in ('csv', 'time', 'dbcpu'):
                            pathlib.Path(f'{stem}.{suffix}').write_bytes(pathlib.Path(f'{attempt_stem}.{suffix}').read_bytes())
                print(phase, rep, backend, appends, fragment, "exit", code, flush=True)

                if code != 0:
                    sys.exit(code)
