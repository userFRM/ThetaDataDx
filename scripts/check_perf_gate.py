#!/usr/bin/env python3
"""Gate per-binding performance on deterministic allocation counts.

The Rust core already has a Criterion wall-clock regression gate
(`scripts/check_bench_regression.py`). This companion gate watches a
metric that a wall-clock gate cannot watch safely on shared CI runners:
allocations per decoded row.

WHY THIS GATE IS SAFE ON HETEROGENEOUS RUNNERS
----------------------------------------------
Every metric this script reads is an allocation COUNT, never a
wall-clock time. Decoding the identical fixture performs the identical
number of heap allocations on a 2-vCPU hosted runner and on a developer
workstation: CPU frequency, scheduler pressure, and memory bandwidth do
not change how many times the decode path calls `alloc`. The figure is
therefore a property of the code, not of the machine, so it can be
pinned to a committed baseline and run anywhere without flaking. A
wall-clock baseline, by contrast, tracks the runner's clock and would
trip on slower hardware alone.

The count is produced by the `bench_decode_allocations` harness, which
installs a counting global allocator ONLY in that bench binary (the
shipped library allocator is unchanged) and writes the per-row figures
to `target/perf-gate/decode_allocations.json`. This script diffs that
file against `crates/thetadatadx/benches/baseline/perf_gate.json`.

GATE SHAPE (mirrors check_bench_regression.py)
----------------------------------------------
A tracked metric fails (non-zero exit) when its allocations-per-unit has
risen above the baseline by more than `--threshold` percent AND by more
than an absolute floor of `--abs-floor` allocations-per-unit. The two
conditions are ANDed for the same reason the wall-clock gate ANDs its
percentage and its nanosecond floor: a purely relative gate is
meaningless when the baseline approaches zero. A future zero-allocation
decode path would have a baseline near 0.0; any positive count is an
infinite percentage over it, so without the floor a single incidental
allocation would trip the gate. Requiring the absolute rise to clear a
small floor keeps the gate sensitive to a real regression (re-cloning a
field per row adds ~1 alloc/row, far above the floor) while refusing to
fire on a rounding-scale move.

Because the metric is exact (an integer count divided by a fixed unit
count, with no measurement jitter), the threshold can be tight. The
default `--threshold` is deliberately small.

Tracked metrics come from the baseline file itself: adding an entry
opts a metric into the gate, removing it opts out. Keys prefixed with
`_` (`_meta`, `_captured_on`) are documentation-only and skipped.

Baseline JSON shape::

    {
        "<metric_id>": {
            "allocs_per_unit": <float>,
            "metric_file": "<file under --metrics-dir>"
        },
        ...
    }

Each `metric_file` is a harness-written report keyed by the same
`<metric_id>`, each value carrying at least `allocs_per_unit`.

Run from the repo root after the bench has written its metric file::

    python3 scripts/check_perf_gate.py
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


def load_json(path: pathlib.Path, what: str) -> dict:
    if not path.is_file():
        sys.stderr.write(
            f"{what} missing at {path}; "
            "run the perf-gate bench first (see the workflow / module header)\n"
        )
        sys.exit(2)
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        sys.stderr.write(f"{what} at {path} is not valid JSON: {exc}\n")
        sys.exit(2)


def load_metric(metrics_dir: pathlib.Path, metric_file: str, metric_id: str) -> float | None:
    """Read `allocs_per_unit` for `metric_id` from a harness report.

    Returns None when the report or the entry is absent so the caller
    can collect a precise "did the bench run?" diagnostic rather than
    failing on the first gap.
    """
    report_path = metrics_dir / metric_file
    if not report_path.is_file():
        return None
    try:
        report = json.loads(report_path.read_text())
    except json.JSONDecodeError:
        return None
    entry = report.get(metric_id)
    if not isinstance(entry, dict):
        return None
    value = entry.get("allocs_per_unit")
    return float(value) if isinstance(value, (int, float)) else None


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Gate decode allocations-per-row against a committed baseline.",
    )
    parser.add_argument(
        "--baseline",
        default="crates/thetadatadx/benches/baseline/perf_gate.json",
        type=pathlib.Path,
        help="committed baseline JSON to diff against",
    )
    parser.add_argument(
        "--metrics-dir",
        default="target/perf-gate",
        type=pathlib.Path,
        help="directory the perf-gate bench wrote its metric report(s) into",
    )
    parser.add_argument(
        "--threshold",
        default=10.0,
        type=float,
        help="fail on an allocations-per-unit rise exceeding this percentage",
    )
    parser.add_argument(
        "--abs-floor",
        default=0.25,
        type=float,
        help=(
            "minimum absolute allocations-per-unit rise required alongside "
            "the percentage threshold; filters rounding-scale moves and "
            "makes the gate meaningful when a baseline approaches zero"
        ),
    )
    args = parser.parse_args()

    baseline = load_json(args.baseline, "baseline file")
    failures: list[tuple[str, float, float, float]] = []
    missing: list[str] = []
    checked = 0

    for metric_id, entry in sorted(baseline.items()):
        # Underscore-prefixed keys are documentation-only (`_meta`,
        # `_captured_on`) — skip them so the loader can host
        # human-readable context alongside the gated samples.
        if metric_id.startswith("_"):
            continue
        metric_file = entry.get("metric_file")
        baseline_value = entry.get("allocs_per_unit")
        if not isinstance(metric_file, str) or not isinstance(
            baseline_value, (int, float)
        ):
            sys.stderr.write(
                f"malformed baseline entry for {metric_id!r}; "
                "expected `metric_file` (str) + `allocs_per_unit` (number)\n"
            )
            return 2

        current = load_metric(args.metrics_dir, metric_file, metric_id)
        if current is None:
            missing.append(metric_id)
            continue

        checked += 1
        abs_delta = current - float(baseline_value)
        # Guard the division: a zero baseline yields an infinite
        # percentage for any positive count, which the absolute floor
        # is there to arbitrate.
        if baseline_value != 0:
            delta_pct = abs_delta / float(baseline_value) * 100.0
        else:
            delta_pct = float("inf") if abs_delta > 0 else 0.0
        over_pct = delta_pct > args.threshold
        regressed = over_pct and abs_delta > args.abs_floor
        # `NOISE` marks a metric that cleared the percentage gate but not
        # the absolute floor — a rounding-scale move we refuse to treat
        # as real.
        status = "REGRESSION" if regressed else ("NOISE" if over_pct else "OK")
        print(
            f"{status:<11} {metric_id:<44} "
            f"baseline {baseline_value:>8.4f}  "
            f"current {current:>8.4f} allocs/unit  "
            f"delta {delta_pct:+.2f}%"
        )
        if regressed:
            failures.append((metric_id, float(baseline_value), current, delta_pct))

    if missing:
        sys.stderr.write(
            "missing perf-gate metric data for the following tracked "
            "metrics (did the bench run and write to --metrics-dir?):\n"
        )
        for metric_id in missing:
            sys.stderr.write(f"  - {metric_id}\n")
        return 2

    if checked == 0:
        sys.stderr.write(
            "no tracked metrics were checked; the baseline lists none "
            "(or all are documentation-only). Add at least one metric entry.\n"
        )
        return 2

    if failures:
        sys.stderr.write(
            f"\n{len(failures)} metric(s) rose beyond the "
            f"{args.threshold:.1f}% threshold:\n"
        )
        for metric_id, base, cur, delta in failures:
            sys.stderr.write(
                f"  - {metric_id}: {base:.4f} -> {cur:.4f} allocs/unit  ({delta:+.2f}%)\n"
            )
        sys.stderr.write(
            "Refresh the baseline only after auditing the rise: confirm the "
            "extra allocations are intended, re-run the bench on a green "
            "main, copy the new figures into the baseline JSON, and commit "
            "that change in its own PR.\n"
        )
        return 1

    print(f"\nAll {checked} tracked metric(s) within threshold.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
