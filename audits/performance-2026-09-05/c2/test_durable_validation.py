import json
import pathlib
import tempfile
import unittest
from unittest import mock

import quiet_host
import summarize
from durable_validation import validate_durable_output
from test_quiet_host import VALID_DURABLE


class DurableValidationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.stem = pathlib.Path(self.directory.name) / 'baseline-postgres-n128-f16-r1'
        self.path = pathlib.Path(f'{self.stem}.csv')

    def validate(self, path):
        validate_durable_output(path, 'postgres', 128, 16)

    def test_complete_expected_case_passes_and_early_return_or_truncation_fails(self):
        self.path.write_text(VALID_DURABLE)
        self.validate(self.path)
        invalid = [
            'postgres,128,16,1,write_failed,append,2,error\ntest result: ok. 1 passed\n',
            'test result: ok. 0 passed\n',
            VALID_DURABLE.replace('versions,129', 'versions,128'),
            VALID_DURABLE.replace('postgres,128,16,1', 'postgres,128,16,2'),
            VALID_DURABLE.replace('postgres,128,16,1', 'mysql,128,16,1'),
            VALID_DURABLE.replace('postgres,128,16,1', 'postgres,512,16,1'),
            VALID_DURABLE.replace('postgres,128,16,1', 'postgres,128,64,1'),
            '\n'.join(line for line in VALID_DURABLE.splitlines() if ',restart,' not in line),
            VALID_DURABLE.replace('restart,versions_digest,0123456789abcdef', 'restart,versions_digest,ffffffffffffffff'),
            VALID_DURABLE.replace('test result: ok.', 'test result: FAILED.'),
            VALID_DURABLE + 'postgres,128,16,1,write_failed,append,2,error\n',
        ]
        for text in invalid:
            with self.subTest(text=text):
                self.path.write_text(text)
                with self.assertRaises(ValueError):
                    self.validate(self.path)

    def test_successful_exit_without_complete_output_is_discarded_and_not_resumable(self):
        quiet = {'load': [1.0], 'eligible': True, 'heavy_processes': [], 'inspection_error': '', 'inspection_returncode': 0}

        def snapshot(path):
            pathlib.Path(path).write_text(json.dumps(quiet))
            return quiet.copy()

        def workload(command, stdout, stderr, env):
            stdout.write('test result: ok. 1 passed\n')
            stderr.write('timing\n')
            return 0

        with mock.patch.object(quiet_host, 'snapshot', side_effect=snapshot), \
             mock.patch.object(quiet_host.subprocess, 'call', side_effect=workload):
            self.assertEqual(quiet_host.run(self.stem, ['mocked'], output_validator=self.validate), 1)
        self.assertTrue(pathlib.Path(f'{self.stem}.attempt1.after.json').exists())
        discarded = pathlib.Path(f'{self.stem}.attempt1.discarded')
        self.assertIn('output validation failed', discarded.read_text())
        self.assertFalse(self.path.exists())
        # Even legacy promoted exit-zero files cannot bypass validation on resume.
        discarded.unlink()
        for suffix in ['csv', 'time']:
            pathlib.Path(f'{self.stem}.{suffix}').write_bytes(pathlib.Path(f'{self.stem}.attempt1.{suffix}').read_bytes())
        with self.assertRaises(ValueError):
            quiet_host.resume_existing(self.stem, True, output_validator=self.validate)

    def test_summarizer_marks_early_return_and_missing_equivalence_failed(self):
        directory = pathlib.Path(self.directory.name) / 'durable'
        directory.mkdir()
        for rep, text in enumerate([
            'postgres,128,16,1,write_failed,append,2,error\ntest result: ok. 1 passed\n',
            'test result: ok. 1 passed\n', VALID_DURABLE,
        ], 1):
            (directory / f'baseline-postgres-n128-f16-r{rep}.csv').write_text(text)
        with mock.patch.object(summarize, 'ROOT', pathlib.Path(self.directory.name)):
            runs, failures = summarize.parse_durable()
        self.assertEqual(len(failures), 2)
        rows = runs[('baseline', 'postgres', 128, 16)]
        self.assertEqual([row.get('failed', False) for row in rows], [True, True, False])
        self.assertEqual(summarize.median(rows, 'versions'), 129)


if __name__ == '__main__':
    unittest.main()
