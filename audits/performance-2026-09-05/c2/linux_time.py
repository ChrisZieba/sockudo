#!/usr/bin/env python3
"""Linux timing adapter: retain C2 time-file names and bytes-based RSS units."""
import os
import resource
import signal
import subprocess
import sys
import time


def main(command):
    if not command:
        raise ValueError('a workload command is required')
    started = time.monotonic()
    process = subprocess.Popen(command, start_new_session=True)
    timeout = int(os.environ.get('C2_WORKLOAD_TIMEOUT_SECONDS', '900'))
    try:
        code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        print(f'workload timed out after {timeout} seconds', file=sys.stderr)
        code = 124
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    print(f'{time.monotonic() - started:.6f} real {usage.ru_utime:.6f} user '
          f'{usage.ru_stime:.6f} sys', file=sys.stderr)
    print(f'{usage.ru_maxrss * 1024} maximum resident set size', file=sys.stderr)
    print('timing_platform linux; source getrusage; rss_unit bytes', file=sys.stderr)
    return code if code >= 0 else 128 - code


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
