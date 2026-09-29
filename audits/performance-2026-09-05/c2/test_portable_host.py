"""Portable commands and Linux evidence must preserve the strict quiet gate."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

import quiet_host
import linux_time


class PortableHostTests(unittest.TestCase):
    def test_platform_commands_preserve_macos_and_select_linux(self):
        with patch.object(quiet_host.platform, 'system', return_value='Darwin'):
            self.assertEqual(quiet_host.time_command(['bin', 'arg']), ['/usr/bin/time', '-l', 'bin', 'arg'])
            self.assertEqual(quiet_host.process_command()[-1], '-r')
        with patch.object(quiet_host.platform, 'system', return_value='Linux'):
            self.assertEqual(quiet_host.time_command(['bin'])[-1], 'bin')
            self.assertEqual(quiet_host.process_command()[-1], '--sort=-pcpu')

    def test_linux_steal_must_be_unchanged_for_accepted_evidence(self):
        before = {'eligible': True, 'load': [0.5], 'heavy_processes': [],
                  'inspection_error': '', 'inspection_returncode': 0, 'linux_steal_ticks': 5}
        after = dict(before, workload_pids=[], unrelated_heavy_processes=[], returncode=0)
        self.assertTrue(quiet_host._host_evidence_accepted(before, after))
        for value in [4, 6, None]:
            self.assertFalse(quiet_host._host_evidence_accepted(before, dict(after, linux_steal_ticks=value)))

    def test_linux_inspection_missing_steal_counter_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(quiet_host.platform, 'system', return_value='Linux'), \
             patch.object(quiet_host.os, 'getloadavg', return_value=(0.1, 0.1, 0.1)), \
             patch.object(quiet_host.subprocess, 'run', return_value=types.SimpleNamespace(returncode=0, stdout='PID %CPU COMM\n1 1.0 init\n', stderr='')), \
             patch.object(quiet_host, 'linux_steal_ticks', side_effect=OSError('missing')):
            state = quiet_host.snapshot(Path(directory) / 'host.json')
            self.assertFalse(state['eligible'])
            self.assertIn('missing', state['inspection_error'])

    def test_linux_time_uses_bytes_and_preserves_failure_code(self):
        process = types.SimpleNamespace(wait=lambda timeout: 7)
        usage = types.SimpleNamespace(ru_utime=1.0, ru_stime=0.5, ru_maxrss=42)
        import contextlib
        import io
        stderr = io.StringIO()
        with patch.object(linux_time.subprocess, 'Popen', return_value=process), \
             patch.object(linux_time.resource, 'getrusage', return_value=usage), \
             contextlib.redirect_stderr(stderr):
            self.assertEqual(linux_time.main(['unused']), 7)
        self.assertIn('43008 maximum resident set size', stderr.getvalue())


if __name__ == '__main__':
    unittest.main()
