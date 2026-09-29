#!/usr/bin/env python3
"""Summarize raw C2 memory and durable results into summary.csv / summary.md.

Every value is the median across repetitions; min–max ranges are reported for
latencies. Failed or incomplete runs are listed, never silently dropped.
"""
import os
import collections
import pathlib
import re
import statistics
from durable_validation import validate_durable_output

ROOT = pathlib.Path(os.environ.get("C2_RESULTS_ROOT", pathlib.Path(__file__).parent / "results"))
CASES = [(n, f) for n in (128, 512, 2000) for f in (16, 64, 256)]


def time_file(path):
    text = path.read_text() if path.exists() else ""
    out = {}
    for key, pattern in {
        "real_s": r"([\d.]+) real",
        "user_s": r"([\d.]+) user",
        "sys_s": r"([\d.]+) sys",
        "max_rss_bytes": r"(\d+)\s+maximum resident set size",
    }.items():
        match = re.search(pattern, text)
        if match:
            out[key] = float(match.group(1))
    return out


def result_files(kind):
    """(phase, path): final runs, then the superseded first candidate."""
    for path in sorted((ROOT / kind).glob("*.csv")):
        if ".attempt" in path.stem:
            continue  # preserved separately; only canonical files were accepted
        yield None, path
    for path in sorted((ROOT / "intermediate-after-v1" / kind).glob("*.csv")):
        yield "after-v1", path


def parse_memory():
    runs = collections.defaultdict(list)
    for override, path in result_files("memory"):
        match = re.match(r"(baseline|after|c2|followup)-n(\d+)-f(\d+)-r(\d+)$", path.stem)
        if not match:
            continue
        phase, n, f, rep = override or match.group(1), int(match.group(2)), int(match.group(3)), int(match.group(4))
        row = {"rep": rep}
        for line in path.read_text().splitlines():
            parts = line.split(",")
            if parts[0] == "equivalence":
                row.update(versions=int(parts[2]), data_bytes=int(parts[4]),
                           versions_digest=parts[6], replay_digest=parts[8])
            elif parts[0] == "retained":
                row["live_heap_bytes"] = int(parts[2])
            elif parts[0] == "writes":
                row.update(write_alloc_calls=int(parts[2]), write_alloc_bytes=int(parts[4]),
                           write_cpu_user_s=float(parts[6]), write_cpu_sys_s=float(parts[8]))
            elif parts[0] == "reads":
                row.update(random_read_alloc_bytes=int(parts[2]))
            elif parts[0] == "latency":
                row[f"{parts[1]}_p50"] = int(parts[3])
                row[f"{parts[1]}_p95"] = int(parts[4])
                row[f"{parts[1]}_p99"] = int(parts[5])
        row.update(time_file(path.with_suffix(".time")))
        runs[(phase, "memory", n, f)].append(row)
    return runs


def parse_durable():
    runs = collections.defaultdict(list)
    failures = []
    for override, path in result_files("durable"):
        match = re.match(r"(baseline|after|c2|followup)-(\w+)-n(\d+)-f(\d+)-r(\d+)$", path.stem)
        if not match:
            failures.append((path.name, "not a measured run"))
            continue
        phase, backend = override or match.group(1), match.group(2)
        n, f, rep = int(match.group(3)), int(match.group(4)), int(match.group(5))
        text = path.read_text()
        row = {"rep": rep}
        try:
            validate_durable_output(path, backend, n, f)
        except (OSError, ValueError) as error:
            failures.append((f"{override + '/' if override else ''}{path.name}", str(error)))
            row["failed"] = True
            runs[(phase, backend, n, f)].append(row)
            continue
        for line in text.splitlines():
            parts = line.split(",")
            if len(parts) < 6 or parts[0] != backend:
                continue
            kind = parts[4]
            if kind == "equivalence":
                row.update(versions=int(parts[6]), data_bytes=int(parts[8]),
                           versions_digest=parts[10], replay_digest=parts[12])
            elif kind == "restart":
                row.update(restart_versions_digest=parts[6], restart_replay_digest=parts[8])
            elif kind == "writes":
                row["write_total_ms"] = int(parts[6])
            elif kind == "latency":
                row[f"{parts[5]}_p50"] = int(parts[7])
                row[f"{parts[5]}_p95"] = int(parts[8])
                row[f"{parts[5]}_p99"] = int(parts[9])
            elif kind == "retained":
                row[parts[5]] = int(parts[6])
            elif kind == "write_volume":
                row[parts[5]] = int(parts[6])
                if len(parts) > 8:
                    row[f"{parts[5]}_per_append"] = int(parts[8])
            elif kind == "write_failed":
                row["write_failed"] = ",".join(parts[5:])
        dbcpu = path.with_suffix(".dbcpu")
        if dbcpu.exists():
            row["db_cpu_s"] = int(dbcpu.read_text().split(",")[1]) / 1e6
        row.update(time_file(path.with_suffix(".time")))
        if "test result: ok" not in text:
            reason = row.get("write_failed") or (re.findall(r"panicked at .*\n(.*)", text) or ["did not pass"])[0]
            failures.append((f"{override + '/' if override else ''}{path.name}", reason[:200]))
            row["failed"] = True
        runs[(phase, backend, n, f)].append(row)
    return runs, failures


def median(rows, key):
    values = [row[key] for row in rows if key in row and not row.get("failed")]
    return statistics.median(values) if values else None


def span(rows, key):
    values = [row[key] for row in rows if key in row and not row.get("failed")]
    return (min(values), max(values)) if values else None


def fmt(value, scale=1, digits=1):
    if value is None:
        return "—"
    value = value / scale
    return f"{value:,.{digits}f}" if digits else f"{value:,.0f}"


def main():
    memory = parse_memory()
    durable, failures = parse_durable()
    all_runs = {**memory, **durable}
    keys = sorted({key for rows in all_runs.values() for row in rows for key in row})
    with open(ROOT / "summary.csv", "w") as out:
        out.write("phase,backend,appends,fragment_bytes,rep," + ",".join(k for k in keys if k != "rep") + "\n")
        for (phase, backend, n, f), rows in sorted(all_runs.items()):
            for row in sorted(rows, key=lambda r: r["rep"]):
                out.write(f"{phase},{backend},{n},{f},{row['rep']},"
                          + ",".join(str(row.get(k, "")) for k in keys if k != "rep") + "\n")

    if "C2_RESULTS_ROOT" in os.environ:
        lines = ["# C2 follow-up measurements", "", "Only accepted processes are included. Rejected attempts remain in the raw logs.", "",
                 "| Phase | Backend | Appends × bytes | Runs | Append p50 ns | Version-read p50 ns | Retained bytes |",
                 "|---|---|---|---:|---:|---:|---:|"]
        for (phase, backend, n, f), rows in sorted(all_runs.items()):
            retained = "live_heap_bytes" if backend == "memory" else "total_logical_bytes"
            lines.append(f"| {phase} | {backend} | {n} × {f} | {len(rows)} | {fmt(median(rows, 'append_ns_p50'))} | {fmt(median(rows, 'random_version_read_ns_p50'))} | {fmt(median(rows, retained), digits=0)} |")
        if not all_runs:
            lines += ["", "No eligible measurements have completed."]
        lines += ["", "## Rejected attempts", ""]
        lines += [f"- `{path.relative_to(ROOT)}`: {path.read_text().strip()}" for path in sorted(ROOT.rglob("*.discarded"))]
        lines += ["", "## Workload failures", ""]
        lines += [f"- `{name}`: {reason}" for name, reason in failures] or ["- none"]
        (ROOT / "summary.md").write_text("\n".join(lines) + "\n")
        return

    lines = [
        "# C2 summary (medians across repetitions)",
        "",
        "Phases: `baseline` = unchanged source (+ prerequisite read fixes); `v1` = first",
        "candidate (superseded: one extra database round trip per SQL read and per",
        "DynamoDB/ScyllaDB/SurrealDB append, slower memory reconstruction copy);",
        "`final` = final source. Baseline and v1 ran on a quiet host. The final",
        "durable pass and the final memory pass overlapped a CPU-heavy interactive",
        "application on the host (load average ~8), so their latencies are not",
        "comparable; retained bytes are deterministic and are taken from `final`.",
        "",
    ]
    for backend in ["memory"] + sorted({key[1] for key in durable}):
        retained_key = "live_heap_bytes" if backend == "memory" else "total_logical_bytes"
        lines += [
            f"## {backend}",
            "",
            "| appends × bytes | runs b/v1/final | retained bytes baseline → final | ratio | versions digest | append p50 µs b → v1 (final*) | append p99 µs b → v1 | random version p50 µs b → v1 (final*) | 100-version page p50 µs b → v1 (final*) | replay-100 p50 µs b → v1 |",
            "|---|---|---|---|---|---|---|---|---|---|",
        ]
        for n, f in CASES:
            base = all_runs.get(("baseline", backend, n, f), [])
            v1 = all_runs.get(("after-v1", backend, n, f), [])
            fin = all_runs.get(("after", backend, n, f), [])
            if not (base or v1 or fin):
                continue
            ok = lambda rows: [row for row in rows if not row.get("failed")]
            ret_after = median(fin, retained_key) if ok(fin) else median(v1, retained_key)
            ret_before = median(base, retained_key)
            ratio = f"{ret_before / ret_after:,.1f}×" if ret_before and ret_after else "—"
            digests = {row.get("versions_digest") for row in ok(base) + ok(v1) + ok(fin) if "versions_digest" in row}
            after_digests = {row.get("versions_digest") for row in ok(v1) + ok(fin) if "versions_digest" in row}
            base_digests = {row.get("versions_digest") for row in ok(base) if "versions_digest" in row}
            if len(digests) == 1:
                digest = "equal"
            elif len(after_digests) == 1 and base_digests:
                digest = "baseline differs (baseline truncated)"
            else:
                digest = "differs"
            src = "" if ok(fin) else " (v1)"
            us = lambda rows, key: fmt(median(rows, key), 1000)
            lines.append(
                f"| {n} × {f} | {len(ok(base))}/{len(base)}, {len(ok(v1))}/{len(v1)}, {len(ok(fin))}/{len(fin)} "
                f"| {fmt(ret_before, 1, 0)} → {fmt(ret_after, 1, 0)}{src} | {ratio} | {digest} "
                f"| {us(base, 'append_ns_p50')} → {us(v1, 'append_ns_p50')} ({us(fin, 'append_ns_p50')}) "
                f"| {us(base, 'append_ns_p99')} → {us(v1, 'append_ns_p99')} "
                f"| {us(base, 'random_version_read_ns_p50')} → {us(v1, 'random_version_read_ns_p50')} ({us(fin, 'random_version_read_ns_p50')}) "
                f"| {us(base, 'page100_read_ns_p50')} → {us(v1, 'page100_read_ns_p50')} ({us(fin, 'page100_read_ns_p50')}) "
                f"| {us(base, 'replay100_read_ns_p50')} → {us(v1, 'replay100_read_ns_p50')} |")
        lines.append("")
    lines.append("\\* final latency overlapped host load; shown only for completeness.")
    lines.append("")
    lines += ["## Failed or incomplete runs", ""]
    lines += [f"- `{name}`: {reason}" for name, reason in failures] or ["- none"]
    (ROOT / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
