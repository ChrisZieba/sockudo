"""Optional strict host gate for C2 measurements; preserves rejected attempts."""
import json
import os
import pathlib
import re
import subprocess
import time


def snapshot(path):
    load = os.getloadavg()
    uptime = subprocess.run(['uptime'], capture_output=True, text=True)
    processes = subprocess.run(['ps', '-A', '-o', 'pid,pcpu,comm', '-r'], capture_output=True, text=True)
    heavy = []
    if processes.returncode == 0:
        for line in processes.stdout.splitlines()[1:]:
            parts = line.split(None, 2)
            if len(parts) == 3:
                try:
                    if float(parts[1]) >= 50:
                        heavy.append(line)
                except ValueError:
                    pass
    state = {'load': load, 'uptime': uptime.stdout, 'top_cpu': processes.stdout.splitlines()[:26],
             'heavy_processes': heavy, 'inspection_error': processes.stderr,
             'inspection_returncode': processes.returncode,
             'eligible': load[0] < 2 and not heavy and processes.returncode == 0}
    pathlib.Path(path).write_text(json.dumps(state, indent=2))
    return state


def _host_evidence_accepted(before, after):
    """Reapply the original start/post gates; post-workload load may remain high."""
    if (before.get('eligible') is not True or not before.get('load')
            or not isinstance(before['load'][0], (int, float))
            or not before['load'][0] < 2 or before.get('heavy_processes') != []
            or before.get('inspection_error') != ''
            or before.get('inspection_returncode', 0) != 0):
        return False
    if (after.get('inspection_error') != ''
            or after.get('inspection_returncode', 0) != 0
            or not isinstance(after.get('heavy_processes'), list)
            or not isinstance(after.get('workload_pids', []), list)
            or after.get('unrelated_heavy_processes', []) != []):
        return False
    # Older snapshots omitted both derived fields when no process was heavy.
    # A heavy process still requires an explicit recorded workload PID.
    workload_pids = after.get('workload_pids', [])
    if any(not isinstance(pid, str) or not pid.isdecimal() for pid in workload_pids):
        return False
    for line in after['heavy_processes']:
        if not isinstance(line, str) or not line.split() or line.split()[0] not in workload_pids:
            return False
    return after.get('returncode', 0) == 0


def resume_existing(stem, resume=False, *, extra_output_suffixes=(), output_validator=None):
    """Skip only unchanged canonical outputs promoted from a quiet successful attempt.

    Older accepted runs did not record returncode. Their canonical files were
    copied only after exit zero; require an exact match to that undiscarded
    attempt. New runs also persist returncode in the after snapshot.
    """
    stem = pathlib.Path(stem)
    suffixes = ('csv', 'time', *extra_output_suffixes)
    canonical = [pathlib.Path(f'{stem}.{suffix}') for suffix in suffixes]
    if not any(path.exists() for path in canonical[:2]):
        return False
    if not resume:
        raise FileExistsError(f'accepted results already exist: {stem}; choose a fresh results root or --resume')
    if not all(path.is_file() and path.stat().st_size for path in canonical):
        raise ValueError(f'incomplete accepted output cannot be resumed: {stem}')
    original = [path.read_bytes() for path in canonical]
    for before_path in stem.parent.glob(f'{stem.name}.attempt*.before.json'):
        if not re.search(r'\.attempt[0-9]+\.before\.json$', before_path.name):
            continue
        attempt = str(before_path)[:-len('.before.json')]
        if pathlib.Path(f'{attempt}.discarded').exists():
            continue
        try:
            before = json.loads(before_path.read_text())
            after = json.loads(pathlib.Path(f'{attempt}.after.json').read_text())
            if not isinstance(before, dict) or not isinstance(after, dict):
                continue
            if not _host_evidence_accepted(before, after):
                continue
            matches = all(pathlib.Path(f'{attempt}.{suffix}').read_bytes() == data
                          for suffix, data in zip(suffixes, original))
        except (OSError, ValueError, TypeError, KeyError):
            continue
        if matches:
            if output_validator:
                output_validator(canonical[0])
            return True
    raise ValueError(f'no matching successful quiet attempt for accepted output: {stem}')


def run(stem, command, env=None, *, resume=False, before_workload=None, after_workload=None, output_validator=None):
    """Retain rejected attempts; optional bounded retries allow build load to decay.

    Optional callbacks bracket only the subprocess, after the start gate. The
    after callback returns extra {suffix: bytes} outputs retained per attempt
    and promoted alongside csv/time only when that attempt is accepted.
    """
    if (before_workload is None) != (after_workload is None):
        raise ValueError('workload measurement requires both callbacks')
    attempts = max(1, min(60, int(os.environ.get("C2_QUIET_ATTEMPTS", "3"))))
    retry_seconds = max(1, min(60, float(os.environ.get("C2_QUIET_WAIT_SECONDS", "10"))))
    stem = pathlib.Path(stem)
    if resume_existing(stem, resume, output_validator=output_validator):
        return 0
    previous = [int(match.group(1)) for path in stem.parent.glob(f'{stem.name}.attempt*.before.json')
                if (match := re.search(r'\.attempt(\d+)\.before\.json$', path.name))]
    first = max(previous, default=0) + 1
    for offset in range(attempts):
        attempt = first + offset
        attempt_stem = f'{stem}.attempt{attempt}'
        before = snapshot(f'{attempt_stem}.before.json')
        if not before['eligible']:
            pathlib.Path(f'{attempt_stem}.discarded').write_text('not started: host load >= 2, heavy process, or process inspection unavailable\n')
            snapshot(f'{attempt_stem}.after.json')
            if offset + 1 < attempts:
                time.sleep(retry_seconds)
            continue
        try:
            measurement = before_workload() if before_workload else None
        except Exception:
            snapshot(f'{attempt_stem}.after.json')
            pathlib.Path(f'{attempt_stem}.discarded').write_text('not started: workload measurement failed\n')
            raise
        with open(f'{attempt_stem}.csv', 'w') as stdout, open(f'{attempt_stem}.time', 'w') as stderr:
            code = subprocess.call(command, stdout=stdout, stderr=stderr, env=env)
        measurement_error = None
        extra_outputs = {}
        try:
            extra_outputs = after_workload(measurement) if after_workload else {}
            for suffix, data in extra_outputs.items():
                if not re.fullmatch(r'[a-z_]+', suffix) or suffix in ('csv', 'time', 'discarded'):
                    raise ValueError(f'invalid measurement output suffix: {suffix}')
                pathlib.Path(f'{attempt_stem}.{suffix}').write_bytes(data)
        except Exception as error:
            measurement_error = error
        after = snapshot(f'{attempt_stem}.after.json')
        # The database VM is part of a durable workload, not another heavy app.
        # Explicit process IDs apply only after execution; the start gate stays strict.
        workload_pids = set((env or {}).get('C2_WORKLOAD_PIDS', '').split(',')) - {''}
        unrelated_heavy = [line for line in after.get('heavy_processes', [])
                           if line.split()[0] not in workload_pids]
        after['returncode'] = code
        after['workload_pids'] = sorted(workload_pids)
        after['unrelated_heavy_processes'] = unrelated_heavy
        pathlib.Path(f'{attempt_stem}.after.json').write_text(json.dumps(after, indent=2))
        if measurement_error is not None:
            pathlib.Path(f'{attempt_stem}.discarded').write_text('discarded: workload measurement failed\n')
            raise measurement_error
        if unrelated_heavy or after.get('inspection_error') or after.get('inspection_returncode', 0):
            pathlib.Path(f'{attempt_stem}.discarded').write_text('discarded: heavy process or unavailable process inspection after workload\n')
            if offset + 1 < attempts:
                time.sleep(retry_seconds)
            continue
        if code:
            pathlib.Path(f'{attempt_stem}.discarded').write_text(f'workload failed: {code}\n')
            return code
        if output_validator:
            try:
                output_validator(pathlib.Path(f'{attempt_stem}.csv'))
            except (OSError, ValueError):
                pathlib.Path(f'{attempt_stem}.discarded').write_text('discarded: workload output validation failed\n')
                return 1
        # Keep original output names/metrics for the existing summarizer.
        for suffix in ('csv', 'time', *extra_outputs):
            pathlib.Path(f'{stem}.{suffix}').write_bytes(pathlib.Path(f'{attempt_stem}.{suffix}').read_bytes())
        return 0
    return 75  # temporary unavailable, never a successful measurement
