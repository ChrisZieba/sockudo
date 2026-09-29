#!/usr/bin/env python3
"""Run the existing AI budget guard on saved binaries under the strict host gate."""
import argparse
import os
import pathlib
import sys

root = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "c2"))
from quiet_host import run

parser = argparse.ArgumentParser()
parser.add_argument("--baseline", required=True, type=pathlib.Path)
parser.add_argument("--followup", required=True, type=pathlib.Path)
parser.add_argument("--output", required=True, type=pathlib.Path)
parser.add_argument("--phases", nargs="+", choices=("baseline", "followup"),
                    default=("baseline", "followup"))
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
statuses = []
for phase in args.phases:
    binary = getattr(args, phase).resolve()
    target = (args.output / f"{phase}-target").resolve()
    env = dict(os.environ, AIT_BENCH_BINARY=str(binary), CARGO_TARGET_DIR=str(target))
    env.pop("CRITERION_HOME", None)
    status = run(args.output / phase, ["bash", str(root / "scripts/ai-transport-bench-guard.sh")], env)
    print(phase, "guard exit", status, flush=True)
    statuses.append(status)
raise SystemExit(75 if 75 in statuses else int(any(statuses)))
