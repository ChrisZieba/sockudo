"""Resume checks use only temporary files and mocked workloads/host inspection."""
import contextlib
import io
import json
import os
import pathlib
import runpy
import sys
import tempfile
import types
import unittest
from unittest import mock

import quiet_host

ROOT = pathlib.Path(__file__).resolve().parent
VALID_DURABLE = (
    'postgres,128,16,1,equivalence,versions,129,data_bytes,132096,versions_digest,0123456789abcdef,replay_digest,0123456789abcdef\n'
    'postgres,128,16,1,restart,versions_digest,0123456789abcdef,replay_digest,0123456789abcdef\n'
    'test result: ok. 1 passed; 0 failed\n'
)


class ResumeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.stem = pathlib.Path(self.directory.name) / 'baseline-n128-f16-r1'

    def evidence(self, stem=None, *, legacy=False):
        stem = stem or self.stem
        stem.parent.mkdir(parents=True, exist_ok=True)
        before = {'load': [1.0, 3.0, 4.0], 'eligible': True, 'heavy_processes': [],
                  'inspection_error': '', 'inspection_returncode': 0}
        # Post-workload load is allowed; only an explicitly recorded workload PID
        # may be heavy. This must not relax the strict start gate.
        after = {'load': [3.0, 3.0, 4.0], 'eligible': False,
                 'heavy_processes': ['42 80.0 database-vm'], 'workload_pids': ['42'],
                 'unrelated_heavy_processes': [], 'inspection_error': '',
                 'inspection_returncode': 0, 'returncode': 0}
        if legacy:
            before.pop('inspection_returncode')
            after.pop('inspection_returncode')
            after.pop('returncode')
        for suffix, state in [('before', before), ('after', after)]:
            pathlib.Path(f'{stem}.attempt1.{suffix}.json').write_text(json.dumps(state))
        for suffix, data in [('csv', b'original metric bytes\n'), ('time', b'original timing bytes\n')]:
            pathlib.Path(f'{stem}.{suffix}').write_bytes(data)
            pathlib.Path(f'{stem}.attempt1.{suffix}').write_bytes(data)
        return before, after

    def update(self, suffix, state):
        pathlib.Path(f'{self.stem}.attempt1.{suffix}.json').write_text(json.dumps(state))

    def test_default_refuses_overwrite(self):
        self.evidence()
        with self.assertRaises(FileExistsError):
            quiet_host.resume_existing(self.stem)
        with self.assertRaises(FileExistsError):
            quiet_host.run(self.stem, ['unused'])

    def test_verified_current_and_legacy_outputs_resume_without_work(self):
        for legacy in [False, True]:
            with self.subTest(legacy=legacy):
                self.evidence(legacy=legacy)
                self.assertTrue(quiet_host.resume_existing(self.stem, True))
                with mock.patch.object(quiet_host, 'snapshot', side_effect=AssertionError('host inspected')), \
                     mock.patch.object(quiet_host.subprocess, 'call', side_effect=AssertionError('work ran')):
                    self.assertEqual(quiet_host.run(self.stem, ['unused'], resume=True), 0)

    def test_legacy_empty_post_process_list_needs_no_pid_exemption(self):
        before, after = self.evidence(legacy=True)
        after['heavy_processes'] = []
        after.pop('workload_pids')
        after.pop('unrelated_heavy_processes')
        self.update('after', after)
        self.assertTrue(quiet_host.resume_existing(self.stem, True))
        after['heavy_processes'] = ['42 80.0 database-vm']
        self.update('after', after)
        with self.assertRaises(ValueError):
            quiet_host.resume_existing(self.stem, True)

    def test_failed_noisy_or_unverifiable_attempts_do_not_resume(self):
        changes = [
            ('before', 'eligible', False), ('before', 'load', [2.0]),
            ('before', 'heavy_processes', ['42 80.0 database-vm']),
            ('before', 'inspection_error', 'unavailable'), ('before', 'inspection_returncode', 1),
            ('after', 'inspection_error', 'unavailable'), ('after', 'inspection_returncode', 1),
            ('after', 'returncode', 1), ('after', 'unrelated_heavy_processes', ['50 80.0 other']),
            ('after', 'heavy_processes', ['50 80.0 other']), ('after', 'workload_pids', []),
        ]
        for phase, key, value in changes:
            with self.subTest(phase=phase, key=key):
                before, after = self.evidence()
                state = before if phase == 'before' else after
                state[key] = value
                self.update(phase, state)
                with self.assertRaises(ValueError):
                    quiet_host.resume_existing(self.stem, True)

    def test_missing_changed_or_discarded_original_evidence_is_rejected(self):
        for target, contents in [
            ('csv', b'changed'), ('time', b'changed'), ('csv', None),
            ('attempt1.csv', b'changed'), ('attempt1.time', None),
            ('attempt1.after.json', b'invalid json'), ('attempt1.before.json', None),
            ('attempt1.discarded', b'workload failed: 1\n'),
        ]:
            with self.subTest(target=target):
                self.evidence()
                path = pathlib.Path(f'{self.stem}.{target}')
                if contents is None:
                    path.unlink()
                else:
                    path.write_bytes(contents)
                with self.assertRaises(ValueError):
                    quiet_host.resume_existing(self.stem, True)
                if target.endswith('discarded'):
                    path.unlink()

    def test_rejected_attempt_without_canonicals_is_rerun_and_preserved(self):
        before, after = self.evidence()
        for suffix in ['csv', 'time']:
            pathlib.Path(f'{self.stem}.{suffix}').unlink()
        pathlib.Path(f'{self.stem}.attempt1.discarded').write_text('failed\n')
        self.assertFalse(quiet_host.resume_existing(self.stem, True))

        def snapshot(path):
            state = before.copy()
            pathlib.Path(path).write_text(json.dumps(state))
            return state

        def workload(command, stdout, stderr, env):
            stdout.write('new result\n')
            stderr.write('new timing\n')
            return 0

        with mock.patch.object(quiet_host, 'snapshot', side_effect=snapshot), \
             mock.patch.object(quiet_host.subprocess, 'call', side_effect=workload):
            self.assertEqual(quiet_host.run(self.stem, ['mocked'], resume=True), 0)
        self.assertEqual(pathlib.Path(f'{self.stem}.attempt1.discarded').read_text(), 'failed\n')
        self.assertEqual(json.loads(pathlib.Path(f'{self.stem}.attempt2.after.json').read_text())['returncode'], 0)
        self.assertTrue(quiet_host.resume_existing(self.stem, True))

    def test_runner_skips_preserve_dbcpu_without_database_or_workload_calls(self):
        for kind in ['memory', 'durable']:
            with self.subTest(kind=kind):
                directory = pathlib.Path(self.directory.name) / kind
                name = 'baseline-n128-f16-r1' if kind == 'memory' else 'baseline-postgres-n128-f16-r1'
                stem = directory / name
                self.evidence(stem)
                if kind == 'durable':
                    for suffix in ['csv', 'attempt1.csv']:
                        pathlib.Path(f'{stem}.{suffix}').write_text(VALID_DURABLE)
                cpu = pathlib.Path(f'{stem}.dbcpu')
                cpu.write_bytes(b'original database CPU\n')
                pathlib.Path(f'{stem}.attempt1.dbcpu').write_bytes(cpu.read_bytes())
                argv = [str(ROOT / f'run_{kind}.py'), 'never-run', 'baseline', '1']
                if kind == 'durable':
                    argv += ['postgres', '1', '128:16']
                argv += ['--resume']
                with mock.patch.object(sys, 'argv', argv), \
                     mock.patch.dict(os.environ, {'C2_RESULTS_ROOT': self.directory.name, 'C2_CASES': '128:16', 'C2_FIRST_REP': '1'}), \
                     mock.patch.object(quiet_host.subprocess, 'run', side_effect=AssertionError('database queried')), \
                     mock.patch.object(quiet_host.subprocess, 'call', side_effect=AssertionError('workload ran')), \
                     contextlib.redirect_stdout(io.StringIO()):
                    runpy.run_path(argv[0], run_name='__main__')
                self.assertEqual(cpu.read_bytes(), b'original database CPU\n')

    def test_cpu_metrics_exclude_waits_and_promote_only_the_accepted_attempt(self):
        quiet = {'load': [1.0], 'eligible': True, 'heavy_processes': [],
                 'inspection_error': '', 'inspection_returncode': 0}
        loaded = dict(quiet, load=[3.0], eligible=False)
        noisy = dict(quiet, heavy_processes=['99 80.0 unrelated'], eligible=False)
        snapshots = iter([loaded, loaded, quiet, noisy, quiet, quiet])
        cpu_values = iter([100, 800, 10000, 10040])
        samples = []

        def snapshot(path):
            state = next(snapshots).copy()
            pathlib.Path(path).write_text(json.dumps(state))
            return state

        def cpu():
            value = next(cpu_values)
            samples.append(value)
            return value

        def workload(command, stdout, stderr, env):
            stdout.write('result\n')
            stderr.write('timing\n')
            return 0

        with mock.patch.object(quiet_host, 'snapshot', side_effect=snapshot), \
             mock.patch.object(quiet_host.subprocess, 'call', side_effect=workload) as process, \
             mock.patch.object(quiet_host.time, 'sleep') as wait, \
             mock.patch.dict(os.environ, {'C2_QUIET_ATTEMPTS': '3'}):
            code = quiet_host.run(
                self.stem, ['mocked'], before_workload=cpu,
                after_workload=lambda before: {'dbcpu': f'container_cpu_usec_delta,{cpu() - before}\n'.encode()},
            )
        self.assertEqual(code, 0)
        self.assertEqual(process.call_count, 2)
        self.assertEqual(wait.call_count, 2)
        self.assertEqual(samples, [100, 800, 10000, 10040])
        self.assertFalse(pathlib.Path(f'{self.stem}.attempt1.dbcpu').exists())
        self.assertEqual(pathlib.Path(f'{self.stem}.attempt2.dbcpu').read_bytes(), b'container_cpu_usec_delta,700\n')
        self.assertTrue(pathlib.Path(f'{self.stem}.attempt2.discarded').exists())
        self.assertEqual(pathlib.Path(f'{self.stem}.dbcpu').read_bytes(), b'container_cpu_usec_delta,40\n')
        self.assertTrue(quiet_host.resume_existing(self.stem, True, extra_output_suffixes=('dbcpu',)))
        pathlib.Path(f'{self.stem}.dbcpu').write_bytes(b'altered CPU\n')
        with self.assertRaises(ValueError):
            quiet_host.resume_existing(self.stem, True, extra_output_suffixes=('dbcpu',))

    def test_orphan_cpu_from_failed_legacy_run_does_not_block_retry(self):
        pathlib.Path(f'{self.stem}.dbcpu').write_bytes(b'failed attempt CPU\n')
        self.assertFalse(quiet_host.resume_existing(self.stem, True, extra_output_suffixes=('dbcpu',)))

    def test_nonquiet_durable_cpu_keeps_the_existing_per_process_delta(self):
        script = ROOT / 'run_durable.py'
        argv = [str(script), 'never-run', 'baseline', '1', 'postgres', '1', '128:16']

        def workload(command, stdout, stderr, env):
            stdout.write(VALID_DURABLE)
            stderr.write('timing\n')
            return 0

        with mock.patch.object(sys, 'argv', argv), \
             mock.patch.dict(os.environ, {'C2_RESULTS_ROOT': self.directory.name, 'C2_QUIET_HOST': ''}), \
             mock.patch.object(quiet_host.subprocess, 'run', side_effect=[
                 types.SimpleNamespace(returncode=0, stdout='usage_usec 100\n'),
                 types.SimpleNamespace(returncode=0, stdout='usage_usec 180\n'),
             ]) as cpu, \
             mock.patch.object(quiet_host.subprocess, 'call', side_effect=workload), \
             contextlib.redirect_stdout(io.StringIO()):
            runpy.run_path(str(script), run_name='__main__')
        self.assertEqual(cpu.call_count, 2)
        metric = pathlib.Path(self.directory.name) / 'durable/baseline-postgres-n128-f16-r1.dbcpu'
        self.assertEqual(metric.read_bytes(), b'container_cpu_usec_delta,80\n')

    def test_invalid_cpu_evidence_retains_host_snapshots_and_never_promotes(self):
        script = ROOT / 'run_durable.py'
        good = types.SimpleNamespace(returncode=0, stdout='usage_usec 100\n')
        cases = [
            [types.SimpleNamespace(returncode=1, stdout='usage_usec 100\n')],
            [types.SimpleNamespace(returncode=0, stdout='missing counter\n')],
            [types.SimpleNamespace(returncode=0, stdout='usage_usec -1\n')],
            [good, types.SimpleNamespace(returncode=0, stdout='usage_usec 99\n')],
        ]
        quiet = {'load': [1.0], 'eligible': True, 'heavy_processes': [], 'inspection_error': '', 'inspection_returncode': 0}
        for index, counters in enumerate(cases):
            with self.subTest(case=index):
                output = pathlib.Path(self.directory.name) / str(index)
                argv = [str(script), 'never-run', 'baseline', '1', 'postgres', '1', '128:16']

                def snapshot(path):
                    pathlib.Path(path).write_text(json.dumps(quiet))
                    return quiet.copy()

                def workload(command, stdout, stderr, env):
                    stdout.write(VALID_DURABLE)
                    stderr.write('timing\n')
                    return 0

                with mock.patch.object(sys, 'argv', argv), \
                     mock.patch.dict(os.environ, {'C2_RESULTS_ROOT': str(output), 'C2_QUIET_HOST': '1'}), \
                     mock.patch.object(quiet_host, 'snapshot', side_effect=snapshot), \
                     mock.patch.object(quiet_host.subprocess, 'run', side_effect=counters), \
                     mock.patch.object(quiet_host.subprocess, 'call', side_effect=workload) as process:
                    with self.assertRaisesRegex(RuntimeError, 'backend=postgres returncode='):
                        runpy.run_path(str(script), run_name='__main__')
                stem = output / 'durable/baseline-postgres-n128-f16-r1'
                for suffix in ['before.json', 'after.json', 'discarded']:
                    self.assertTrue(pathlib.Path(f'{stem}.attempt1.{suffix}').exists())
                for suffix in ['csv', 'time', 'dbcpu']:
                    self.assertFalse(pathlib.Path(f'{stem}.{suffix}').exists())
                self.assertEqual(process.call_count, int(len(counters) == 2))
                if process.call_count:
                    self.assertEqual(pathlib.Path(f'{stem}.attempt1.csv').read_text(), VALID_DURABLE)

    def test_interleaved_resume_flag_is_forwarded_to_both_runners(self):
        binaries = pathlib.Path(self.directory.name) / 'binaries.json'
        binaries.write_text(json.dumps({phase: {'memory': 'memory-bin', 'durable': 'durable-bin'}
                                       for phase in ['baseline', 'c2', 'followup']}))
        script = ROOT.parent / 'c2-followup' / 'run_interleaved.py'
        with mock.patch.object(sys, 'argv', [str(script), str(binaries), '--reps', '1', '--resume']), \
             mock.patch.object(quiet_host.subprocess, 'run', return_value=types.SimpleNamespace(returncode=0)) as launch:
            runpy.run_path(str(script), run_name='__main__')
        self.assertEqual(launch.call_count, 6)
        for call in launch.call_args_list:
            self.assertEqual(call.args[0][-1], '--resume')
            self.assertEqual(call.kwargs['env']['C2_QUIET_HOST'], '1')


if __name__ == '__main__':
    unittest.main()
