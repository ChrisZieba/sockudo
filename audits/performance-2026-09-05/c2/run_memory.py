#!/usr/bin/env python3
"""Run the C2 memory diagnostic matrix; keep every raw output.

usage: run_memory.py <binary> <phase> [repetitions]
"""
import argparse
import os
from quiet_host import resume_existing, run, time_command
import pathlib
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary')
parser.add_argument('phase')
parser.add_argument('repetitions', type=int, nargs='?', default=5)
parser.add_argument('--resume', action='store_true', help='reuse verified, unchanged accepted results')
args = parser.parse_args()
binary, phase, reps = args.binary, args.phase, args.repetitions
out = pathlib.Path(os.environ.get("C2_RESULTS_ROOT", pathlib.Path(__file__).parent / "results")) / "memory"
out.mkdir(parents=True, exist_ok=True)
for rep in range(int(os.environ.get("C2_FIRST_REP", "1")), reps + 1):
    for appends in (128, 512, 2000):
        for fragment in (16, 64, 256):
            cases = os.environ.get("C2_CASES")
            if cases and f"{appends}:{fragment}" not in cases.split(","):
                continue
            stem = out / f"{phase}-n{appends}-f{fragment}-r{rep}"
            if resume_existing(stem, args.resume):
                print(phase, rep, appends, fragment, "resume accepted", flush=True)
                continue
            command = time_command([binary, str(appends), str(fragment)])
            if os.environ.get("C2_QUIET_HOST"):
                code = run(stem, command)
            else:
                with open(f"{stem}.csv", "w") as stdout, open(f"{stem}.time", "w") as stderr:
                    code = subprocess.call(command, stdout=stdout, stderr=stderr)
            print(phase, rep, appends, fragment, "exit", code, flush=True)
            if code != 0:
                sys.exit(code)
