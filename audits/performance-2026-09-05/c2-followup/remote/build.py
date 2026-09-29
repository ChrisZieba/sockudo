#!/usr/bin/env python3
"""Build one immutable benchmark source on CI; never run measurements here."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

BASE = '3b57039d4565a01a1bdae66fd891872e7e49debd'
TREE = '574ca6a1895453ad97ec555abc1f7a424f4418b8'
PATCH_SHA = '588119b5b3dd415b0a2a381c5ddda20632054f676491dc4a33727d77b6ac87bf'
HISTORY = Path('crates/sockudo-server/src/history')
MEMORY = Path('crates/sockudo-core/examples/c2_append_storage.rs')
FEATURES = 'local,versioned-messages,postgres,mysql,dynamodb,scylladb,surrealdb'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f'expected one diagnostic overlay anchor: {old[:65]}')
    return text.replace(old, new, 1)


def memory_harness(text):
    # getrusage has different units/types on Linux; macOS output is unchanged.
    text = replace_once(text, 'f64::from(t.tv_usec)', 't.tv_usec as f64')
    text = replace_once(text, '''        #[cfg(not(target_os = "macos"))]
        pub tv_usec: i32,
        #[cfg(not(target_os = "macos"))]
        pub _high: i32,''', '''        #[cfg(not(target_os = "macos"))]
        pub tv_usec: i64,''')
    text = replace_once(text, '    (secs(usage.ru_utime), secs(usage.ru_stime), usage.ru_maxrss)',
        '''    let max_rss = if cfg!(target_os = "linux") {
        usage.ru_maxrss * 1024
    } else {
        usage.ru_maxrss
    };
    (secs(usage.ru_utime), secs(usage.ru_stime), max_rss)''')
    return text


def durable_harness(text, historical):
    # Same append/read timings and deterministic fixture for all three builds.
    # Omit unrelated mixed-release/concurrency tests from the CI executable.
    text = text[:text.index('/// Canonical full-state records')] + text[text.index('async fn write_counters('):]
    # Optional progress is outside both timestamp boundaries and is disabled
    # in latency jobs; it can locate a historical stall in a separate diagnosis.
    text = replace_once(text, '    let started_all = Instant::now();',
        '    let append_progress = std::env::var_os("C2_APPEND_PROGRESS").is_some();\n    let started_all = Instant::now();')
    text = replace_once(text,
        '        let started = Instant::now();\n        let outcome = store',
        '        if append_progress { eprintln!("c2_append_start,{}", index + 1); }\n        let started = Instant::now();\n        let outcome = store')
    text = replace_once(text, '        write_ns.push(started.elapsed().as_nanos() as u64);',
        '        write_ns.push(started.elapsed().as_nanos() as u64);\n        if append_progress { eprintln!("c2_append_end,{}", index + 1); }')
    # Volume hooks do not exist on historical sources and are not latency data.
    start = text.index('        #[cfg(feature = "dynamodb")]', text.index('async fn write_counters('))
    end = text.index('        #[cfg(feature = "postgres")]', start)
    text = text[:start] + text[end:]
    if historical:
        text = replace_once(text, '''    if std::env::var_os("C2_CHUNKED").is_some() {
        store.set_append_storage_enabled(true).await.unwrap();
    }
''', '''    assert!(std::env::var_os("C2_CHUNKED").is_none(),
        "historical source must retain its original storage format");
''')
    return text


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def prepare(repo, work, phase, followup):
    source = work / 'source'
    source_ref = followup if phase == 'followup' else BASE
    run(['git', '-C', str(repo), 'worktree', 'add', '--detach', str(source), source_ref])
    patches = {}
    if phase == 'c2':
        patch = repo / 'audits/performance-2026-09-05/c2-followup/source-snapshots/c2-final-from-baseline.patch'
        if sha(patch) != PATCH_SHA:
            raise ValueError('C2 source patch hash mismatch')
        run(['git', 'apply', '--index', str(patch)], cwd=source)
        actual_tree = subprocess.check_output(['git', 'write-tree'], cwd=source, text=True).strip()
        if actual_tree != TREE:
            raise ValueError(f'C2 source tree mismatch: {actual_tree}')
        patches[patch.name] = sha(patch)
    elif phase == 'baseline':
        patch = repo / 'audits/performance-2026-09-05/c2/prerequisite-read-fixes.patch'
        run(['git', 'apply', '--index', str(patch)], cwd=source)
        patches[patch.name] = sha(patch)
    production_tree = subprocess.check_output(['git', 'write-tree'], cwd=source, text=True).strip()
    overlays = {
        MEMORY: memory_harness((repo / MEMORY).read_text()),
        HISTORY / 'c2_bench.rs': durable_harness((repo / HISTORY / 'c2_bench.rs').read_text(), phase != 'followup'),
        HISTORY / 'c2_wire_meter.rs': (repo / HISTORY / 'c2_wire_meter.rs').read_text(),
    }
    module = source / HISTORY / 'mod.rs'
    text = module.read_text()
    for name in ['c2_bench', 'c2_wire_meter']:
        if f'mod {name};' not in text:
            text += f'\n#[cfg(all(test, feature = "versioned-messages"))]\nmod {name};\n'
    overlays[HISTORY / 'mod.rs'] = text
    for path, contents in overlays.items():
        (source / path).parent.mkdir(parents=True, exist_ok=True)
        (source / path).write_text(contents)
    return source, {'phase': phase, 'source_ref': source_ref, 'production_tree': production_tree,
                    'patches': patches, 'harnesses': {str(p): sha(source / p) for p in overlays}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['baseline', 'c2', 'followup'])
    parser.add_argument('work', type=Path)
    parser.add_argument('--repo', type=Path, default=Path.cwd())
    args = parser.parse_args()
    repo, work = args.repo.resolve(), args.work.resolve()
    work.mkdir(parents=True, exist_ok=False)
    output = work / 'artifacts'
    output.mkdir()
    followup = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    source, manifest = prepare(repo, work, args.phase, followup)
    env = dict(os.environ)
    # Explicit profiles and a new per-build target prevent stale artifact reuse.
    for key in list(env):
        if key.startswith(('CARGO_PROFILE_', 'C2_', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS')):
            env.pop(key)
    env.update(CARGO_TARGET_DIR=str(work / 'target'), CARGO_INCREMENTAL='0',
               CARGO_BUILD_JOBS='2', CARGO_PROFILE_RELEASE_LTO='true',
               CARGO_PROFILE_RELEASE_CODEGEN_UNITS='1')
    commands = [
        ['cargo', 'build', '--locked', '-p', 'sockudo-core', '--example', 'c2_append_storage',
         '--release', '--no-default-features', '--features', 'local'],
        ['cargo', 'test', '--locked', '-p', 'sockudo', '--bin', 'sockudo', '--release', '--no-default-features',
         '--features', FEATURES, 'c2_durable_append_storage', '--no-run', '--message-format=json'],
    ]
    build_environment = {'runner_image': os.environ.get('ImageVersion', ''),
                         'system_packages': subprocess.check_output(
                             ['dpkg-query', '-W', 'libssl-dev', 'libpq-dev', 'cmake', 'protobuf-compiler',
                              'libprotobuf-dev', 'clang', 'libc6-dev'], text=True)}
    manifest.update(build_environment=build_environment, followup_commit=followup, rustc=subprocess.check_output(['rustc', '-Vv'], text=True),
                    cargo=subprocess.check_output(['cargo', '-V'], text=True),
                    cargo_lock_sha256=sha(source / 'Cargo.lock'), commands=commands,
                    memory_profile={'lto': True, 'codegen_units': 1},
                    durable_profile={'lto': False, 'codegen_units': 16}, build_jobs=2,
                    cargo_config_sha256=sha(source / '.cargo/config.toml'))
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    with (output / 'memory-build.log').open('w') as log:
        run(commands[0], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT)
    import shutil
    shutil.copy2(work / 'target/release/examples/c2_append_storage', output / 'memory')
    env.update(CARGO_PROFILE_RELEASE_LTO='false', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',
               CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP='false')
    with (output / 'durable-build.jsonl').open('w') as log, (output / 'durable-build.log').open('w') as errors:
        run(commands[1], cwd=source, env=env, stdout=log, stderr=errors)
    binaries = []
    for line in (output / 'durable-build.jsonl').read_text().splitlines():
        event = json.loads(line)
        if (event.get('reason') == 'compiler-artifact' and event.get('executable')
                and event.get('target', {}).get('name') == 'sockudo'
                and event.get('profile', {}).get('test')):
            binaries.append(event['executable'])
    if len(binaries) != 1:
        raise ValueError(f'expected one newly built durable test executable, got {len(binaries)}')
    shutil.copy2(binaries[0], output / 'durable')
    manifest['binaries'] = {kind: sha(output / kind) for kind in ['memory', 'durable']}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
