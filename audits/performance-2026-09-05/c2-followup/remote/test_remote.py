"""Source overlay/provenance and result-gate checks: no compilation or database."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build
import measure
import validate

REPO = Path(__file__).resolve().parents[4]


class OverlayTests(unittest.TestCase):
    def test_historical_harness_only_changes_explicit_marker_branch(self):
        text = (REPO / build.HISTORY / 'c2_bench.rs').read_text()
        historical = build.durable_harness(text, True)
        current = build.durable_harness(text, False)
        self.assertIn('historical source must retain its original storage format', historical)
        self.assertNotIn('set_append_storage_enabled', historical)
        self.assertIn('set_append_storage_enabled', current)
        for anchor in ['fn fragment(', 'fn version(', 'fn create_record(', 'fn digest(']:
            start = historical.index(anchor)
            end = historical.index('\n}\n', start) + 3
            self.assertIn(historical[start:end], current)
        self.assertNotIn('benchmark_write_counters', historical)

    def test_memory_overlay_changes_only_rss_unit_conversion(self):
        text = (REPO / build.MEMORY).read_text()
        changed = build.memory_harness(text)
        self.assertIn('usage.ru_maxrss * 1024', changed)
        self.assertIn('pub tv_usec: i64', changed)
        self.assertNotIn('pub _high', changed)
        self.assertIn('t.tv_usec as f64', changed)
        self.assertNotIn('f64::from(t.tv_usec)', changed)
        self.assertEqual(text[text.index('fn main()'):], changed[changed.index('fn main()'):])

    def test_c2_source_patch_has_the_pinned_hash(self):
        path = REPO / 'audits/performance-2026-09-05/c2-followup/source-snapshots/c2-final-from-baseline.patch'
        self.assertEqual(build.sha(path), build.PATCH_SHA)

    def test_source_anchor_drift_is_rejected(self):
        with self.assertRaises(ValueError):
            build.replace_once('other', 'expected', 'replacement')
        with self.assertRaises(ValueError):
            build.replace_once('aa', 'a', 'replacement')


class EvidenceTests(unittest.TestCase):
    def test_complete_matrix_requires_ninety_verified_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for backend in validate.BACKENDS:
                for n, f in validate.CASES:
                    if backend in ('dynamodb', 'surrealdb') and n == 2000:
                        continue
                    for phase in validate.PHASES:
                        for rep in range(1, 4):
                            kind = 'memory' if backend == 'memory' else 'durable'
                            suffix = '' if kind == 'memory' else f'-{backend}'
                            path = root / kind / f'{phase}{suffix}-n{n}-f{f}-r{rep}.csv'
                            path.parent.mkdir(exist_ok=True)
                            prefix = '' if kind == 'memory' else f'{backend},{n},{f},1,'
                            text = prefix + f'equivalence,versions,{n + 1},data_bytes,0,versions_digest,0123456789abcdef,replay_digest,0123456789abcdef\n'
                            for metric in ['append_ns', 'get_latest_ns', 'random_version_read_ns', 'page100_read_ns', 'replay100_read_ns']:
                                text += prefix + f'latency,{metric},201,100,110,120,130\n'
                            path.write_text(text)
            with patch.object(validate, 'resume_existing', return_value=True) as verified:
                result = validate.validate(root)
                self.assertEqual(result['accepted_processes'], 90)
                self.assertEqual(verified.call_count, 90)
            broken = root / 'memory/followup-n128-f16-r3.csv'
            broken.write_text(broken.read_text().replace('0123456789abcdef', 'fedcba9876543210'))
            with patch.object(validate, 'resume_existing', return_value=True):
                with self.assertRaisesRegex(ValueError, 'cross-build/backend digest mismatch'):
                    validate.validate(root)

    def test_missing_process_cannot_pass_remote_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'missing quiet sample'):
                validate.validate(Path(directory))

    def test_incomplete_memory_digest_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'sample.csv'
            path.write_text('equivalence,versions,129,data_bytes,0,versions_digest,0123456789abcdef,replay_digest,fedcba9876543210\n')
            with self.assertRaisesRegex(ValueError, 'digest invalid'):
                validate.validate_memory(path, 128)

    def test_binary_hash_change_is_rejected_before_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            phase = root / 'baseline'
            phase.mkdir()
            (phase / 'manifest.json').write_text(json.dumps({'phase': 'baseline', 'binaries': {'memory': 'wrong'}}))
            (phase / 'memory').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'binary hash mismatch'):
                measure.verify_artifacts(root)


if __name__ == '__main__':
    unittest.main()
