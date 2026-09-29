#!/usr/bin/env python3
"""Interleave baseline, C2 and followup using the existing measurement runners.

Pass a JSON file mapping baseline/c2/followup to {memory: path, durable: path}.
Only followup enables the persisted format marker. All other settings identical.
"""
import argparse
import json
import os
import pathlib
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('binaries', type=pathlib.Path)
parser.add_argument('--reps', type=int, default=3)
parser.add_argument('--cases', default='128:16,2000:256')
parser.add_argument('--output', type=pathlib.Path, help='results directory (fresh unless --resume)')
parser.add_argument('--resume', action='store_true', help='reuse verified, unchanged accepted results')
parser.add_argument('--backends', default='postgres,mysql,dynamodb,scylladb,surrealdb')
args = parser.parse_args()
binaries = json.loads(args.binaries.read_text())
root = pathlib.Path(__file__).resolve().parent
runners = root.parent / 'c2'
for rep in range(1, args.reps + 1):
    for kind in ('memory', 'durable'):
        for phase in ('baseline', 'c2', 'followup'):
            env = dict(os.environ, C2_QUIET_HOST='1', C2_RESULTS_ROOT=str((args.output or root / 'results').resolve()),
                       C2_FIRST_REP=str(rep), C2_CASES=args.cases)
            for diagnostic in ('C2_CHUNKED', 'C2_WIRE_BYTES', 'C2_WRITE_METRICS', 'C2_WIRE_BUDGETS', 'C2_WIRE_PHASES'):
                env.pop(diagnostic, None)
            if phase == 'followup':
                env['C2_CHUNKED'] = '1'
            command = ['python3', str(runners / f'run_{kind}.py'), binaries[phase][kind], phase, str(rep)]
            if kind == 'durable':
                command += [args.backends, str(rep), args.cases]
            if args.resume:
                command.append("--resume")
            result = subprocess.run(command, env=env)
            if result.returncode:
                raise SystemExit(result.returncode)
