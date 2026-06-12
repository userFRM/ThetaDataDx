#!/usr/bin/env python3
"""Compare Criterion bench results against a checked-in baseline.

After `cargo bench` finishes, Criterion writes per-benchmark
`estimates.json` files under `target/criterion/<group>/<id>/new/`.
This script:

* Loads `benches/baseline/criterion.json` (committed snapshot of
  the last-blessed p50 timings).
* Walks each tracked bench and reads the p50 (`median.point_estimate`)
  from Criterion's `new/estimates.json` for that bench.
* Fails (non-zero exit) when any tracked bench's p50 has regressed
  by more than `--threshold` (default 20%) *and* by more than an
  absolute floor of `--abs-floor-ns` (default 1.0 ns).

The absolute floor exists because a purely relative gate is not
meaningful for sub-nanosecond benches. A match-dispatch bench that
measures ~1 ns on a dedicated box drifts by 0.3-0.4 ns of pure jitter
on a shared CI runner — already +30-40%, enough to trip a 25% relative
gate on noise alone. Requiring the absolute delta to clear a 1 ns floor
keeps the gate sensitive to real regressions on the large benches (a
807 us streaming bench would need ~200 us to fail, far above the floor)
while refusing to gate the timer-resolution band of the fastest ones.

Tracked benches come from the baseline file itself — adding a new
entry there opts a bench into the gate; removing it opts out. The
baseline is refreshed by re-running the bench suite on `main`,
copying the new p50s into `benches/baseline/criterion.json`, and
committing that delta in its own PR.

Baseline JSON shape:
{
    "<bench_id>": {
        "p50_ns": <float>,
        "criterion_path": "target/criterion/<group>/<id>"
    },
    ...
}
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


def load_baseline(path: pathlib.Path) -> dict:
    if not path.is_file():
        sys.stderr.write(
            f"baseline file missing at {path}; "
            "regenerate it from a green main run and commit\n"
        )
        sys.exit(2)
    return json.loads(path.read_text())


def load_current(criterion_root: pathlib.Path, criterion_path: str) -> float | None:
    estimates_path = criterion_root / criterion_path / "new" / "estimates.json"
    if not estimates_path.is_file():
        return None
    data = json.loads(estimates_path.read_text())
    # Criterion's `estimates.json` carries `median.point_estimate`
    # in nanoseconds (per the iteration count reported alongside);
    # this is the p50 we want to gate on.
    median = data.get("median")
    if not isinstance(median, dict):
        return None
    point = median.get("point_estimate")
    return float(point) if isinstance(point, (int, float)) else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline",
        default="crates/thetadatadx/benches/baseline/criterion.json",
        type=pathlib.Path,
        help="committed baseline JSON to diff against",
    )
    parser.add_argument(
        "--criterion-root",
        default="target/criterion",
        type=pathlib.Path,
        help="root directory where Criterion writes its `new/estimates.json`",
    )
    parser.add_argument(
        "--threshold",
        default=20.0,
        type=float,
        help="fail on p50 regression exceeding this percentage",
    )
    parser.add_argument(
        "--abs-floor-ns",
        default=1.0,
        type=float,
        help=(
            "minimum absolute p50 increase (ns) required alongside the "
            "percentage threshold; filters sub-nanosecond runner jitter"
        ),
    )
    args = parser.parse_args()

    baseline = load_baseline(args.baseline)
    failures: list[tuple[str, float, float, float]] = []
    missing: list[str] = []

    for bench_id, entry in sorted(baseline.items()):
        # Underscore-prefixed keys are documentation-only entries
        # (`_meta`, deferred-baseline notes) — skip them silently so the
        # loader can host human-readable context alongside the gated
        # samples without confusing the loop.
        if bench_id.startswith("_"):
            continue
        criterion_path = entry.get("criterion_path")
        baseline_p50 = entry.get("p50_ns")
        if not isinstance(criterion_path, str) or not isinstance(
            baseline_p50, (int, float)
        ):
            sys.stderr.write(
                f"malformed baseline entry for {bench_id!r}; "
                "expected `criterion_path` (str) + `p50_ns` (number)\n"
            )
            return 2

        current_p50 = load_current(args.criterion_root, criterion_path)
        if current_p50 is None:
            missing.append(bench_id)
            continue

        abs_delta = current_p50 - float(baseline_p50)
        delta_pct = abs_delta / float(baseline_p50) * 100.0
        over_pct = delta_pct > args.threshold
        regressed = over_pct and abs_delta > args.abs_floor_ns
        # `NOISE` marks a bench that cleared the percentage gate but not the
        # absolute floor — a sub-nanosecond move we refuse to treat as real.
        status = "REGRESSION" if regressed else ("NOISE" if over_pct else "OK")
        print(
            f"{status:<11} {bench_id:<60} "
            f"baseline {baseline_p50:>12.1f} ns  "
            f"current {current_p50:>12.1f} ns  "
            f"delta {delta_pct:+.2f}%"
        )
        if regressed:
            failures.append((bench_id, float(baseline_p50), current_p50, delta_pct))

    if missing:
        sys.stderr.write(
            "missing Criterion output for the following tracked benches "
            "(did the bench run?):\n"
        )
        for bench_id in missing:
            sys.stderr.write(f"  - {bench_id}\n")
        return 2

    if failures:
        sys.stderr.write(
            f"\n{len(failures)} bench(es) regressed beyond the {args.threshold:.1f}% threshold:\n"
        )
        for bench_id, base, cur, delta in failures:
            sys.stderr.write(
                f"  - {bench_id}: {base:.1f} -> {cur:.1f} ns  ({delta:+.2f}%)\n"
            )
        sys.stderr.write(
            "Refresh the baseline only after auditing the regression: "
            "re-run the bench on a green main, copy the new p50 into "
            "the baseline JSON, and commit that change in its own PR.\n"
        )
        return 1

    print("\nAll tracked benches within threshold.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
