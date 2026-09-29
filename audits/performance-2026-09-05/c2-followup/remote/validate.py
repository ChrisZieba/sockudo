#!/usr/bin/env python3
"""Fail closed unless every three-build/repetition sample has quiet evidence."""
import argparse
import json
from pathlib import Path
import re
import statistics
import sys

RUNNERS = Path(__file__).resolve().parents[2] / 'c2'
sys.path.insert(0, str(RUNNERS))
from quiet_host import resume_existing
from durable_validation import validate_durable_output

PHASES = ('baseline', 'c2', 'followup')
CASES = ((128, 16), (2000, 256))
BACKENDS = ('memory', 'postgres', 'mysql', 'dynamodb', 'scylladb', 'surrealdb')


def validate_memory(path, appends):
    rows = [line.split(',') for line in Path(path).read_text().splitlines()]
    eq = [row for row in rows if row[0] == 'equivalence']
    if (len(eq) != 1 or len(eq[0]) != 9 or eq[0][1::2] !=
            ['versions', 'data_bytes', 'versions_digest', 'replay_digest']
            or eq[0][2] != str(appends + 1) or not eq[0][4].isdigit()
            or not re.fullmatch(r'[0-9a-f]{16}', eq[0][6]) or eq[0][6] != eq[0][8]):
        raise ValueError('memory count or reconstruction digest invalid')
    return eq[0][6]


def validate(output):
    samples = []
    digests = {}
    for backend in BACKENDS:
        for appends, fragment in CASES:
            if backend in ('dynamodb', 'surrealdb') and (appends, fragment) == (2000, 256):
                continue  # unchanged historical compatibility matrix
            kind = 'memory' if backend == 'memory' else 'durable'
            for phase in PHASES:
                for rep in range(1, 4):
                    suffix = '' if kind == 'memory' else f'-{backend}'
                    stem = output / kind / f'{phase}{suffix}-n{appends}-f{fragment}-r{rep}'
                    path = Path(f'{stem}.csv')
                    if kind == 'memory':
                        checker = lambda p: validate_memory(p, appends)
                        extras = ()
                    else:
                        checker = lambda p: validate_durable_output(p, backend, appends, fragment)
                        extras = ('dbcpu',)
                    if not resume_existing(stem, True, extra_output_suffixes=extras, output_validator=checker):
                        raise ValueError(f'missing quiet sample: {stem.name}')
                    rows = [line.split(',') for line in path.read_text().splitlines()]
                    if kind == 'durable':
                        rows = [row[4:] for row in rows if row[:4] == [backend, str(appends), str(fragment), '1']]
                    eq = next(row for row in rows if row[0] == 'equivalence')
                    digest = eq[6]
                    key = (appends, fragment)
                    if key in digests and digests[key] != digest:
                        raise ValueError(f'cross-build/backend digest mismatch: {stem.name}')
                    digests[key] = digest
                    latencies = {row[1]: int(row[3]) for row in rows if row[0] == 'latency'}
                    required = {'append_ns', 'get_latest_ns', 'random_version_read_ns',
                                'page100_read_ns', 'replay100_read_ns'}
                    if set(latencies) != required:
                        raise ValueError(f'incomplete latency samples: {stem.name}')
                    samples.append({'backend': backend, 'appends': appends, 'fragment': fragment,
                                    'phase': phase, 'rep': rep, 'p50_ns': latencies})
    comparisons = []
    for backend in BACKENDS:
        for appends, fragment in CASES:
            group = [s for s in samples if (s['backend'], s['appends'], s['fragment']) == (backend, appends, fragment)]
            if not group:
                continue
            for metric in ['append_ns', 'get_latest_ns', 'random_version_read_ns', 'page100_read_ns', 'replay100_read_ns']:
                values = {phase: [s['p50_ns'][metric] for s in group if s['phase'] == phase] for phase in PHASES}
                medians = {phase: statistics.median(v) for phase, v in values.items()}
                comparisons.append({'backend': backend, 'appends': appends, 'fragment': fragment,
                                    'metric': metric, 'medians_ns': medians,
                                    'ranges_ns': {phase: [min(v), max(v)] for phase, v in values.items()},
                                    'followup_vs_baseline_percent': (medians['followup'] / medians['baseline'] - 1) * 100,
                                    'followup_vs_c2_percent': (medians['followup'] / medians['c2'] - 1) * 100})
    result = {'accepted_processes': len(samples), 'repetitions_per_build_case': 3,
              'all_cross_build_digests_match': True, 'samples': samples, 'comparisons': comparisons,
              'note': 'This validates evidence completeness, not automatic acceptance of latency regressions.'}
    (output / 'latency-validation.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    print(json.dumps({'accepted_processes': validate(args.output)['accepted_processes']}))
