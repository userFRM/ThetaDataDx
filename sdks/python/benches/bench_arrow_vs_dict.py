"""
Benchmark: Arrow columnar DataFrame pipeline wall-clock at increasing N.

Usage::

    # Quick run (all Ns), plain output:
    python benches/bench_arrow_vs_dict.py

    # Detailed pytest-benchmark run (statistics + histogram):
    pytest benches/bench_arrow_vs_dict.py --benchmark-only

The harness constructs N EodTick instances in Python, then times:

1. `to_arrow(ticks)`         -- the public Arrow entry point.
2. `to_dataframe(ticks)`     -- pandas via pyarrow.Table.to_pandas().
3. `to_polars(ticks)`        -- polars via polars.from_arrow().

Also performs an RSS-delta probe around one 100k-row call to validate
the Arrow C Data Interface handoff is zero-copy: RSS growth should be
approximately `N_rows * avg_col_width * n_cols`, not `2x` or `3x` that.

The benchmark does NOT compare against the old dict-of-lists path --
that path has been replaced (see `feat/arrow-columnar-dataframe`). The
old numbers (~300-500ms at 100k rows) are documented for posterity in
the PR description.
"""

from __future__ import annotations

import gc
import os
import resource
import time
from typing import Callable, List

import thetadatadx


def build_eod_ticks(n: int) -> List["thetadatadx.EodTick"]:
    """Construct N EodTick instances with varying values."""
    return [
        thetadatadx.EodTick(
            ms_of_day=i,
            ms_of_day2=i,
            open=float(100 + (i % 1000) * 0.01),
            high=float(101 + (i % 1000) * 0.01),
            low=float(99 + (i % 1000) * 0.01),
            close=float(100 + (i % 1000) * 0.01),
            volume=1_000 * (i + 1),
            count=10 * (i + 1),
            bid_size=10,
            bid_exchange=1,
            bid=float(99.95 + (i % 100) * 0.01),
            bid_condition=0,
            ask_size=20,
            ask_exchange=2,
            ask=float(100.05 + (i % 100) * 0.01),
            ask_condition=0,
            date=20260420,
            expiration=20260517,
            strike=100.0,
            right="C" if i % 2 == 0 else "P",
        )
        for i in range(n)
    ]


def rss_kb() -> int:
    """Return this process's maximum resident set size in kilobytes (Linux)."""
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss


def time_call(fn: Callable[[], object], repeats: int = 3) -> float:
    """Return the best-of-repeats wall-clock time in seconds."""
    best = float("inf")
    for _ in range(repeats):
        gc.collect()
        t0 = time.perf_counter()
        out = fn()
        t1 = time.perf_counter()
        del out
        best = min(best, t1 - t0)
    return best


def run_size(n: int) -> None:
    """Build `n` ticks, time each adapter, print in a consistent format."""
    print(f"\n=== N = {n:,} ticks ===")
    t0 = time.perf_counter()
    ticks = build_eod_ticks(n)
    t1 = time.perf_counter()
    print(f"build Python list:     {(t1 - t0) * 1000:8.1f} ms")

    t_arrow = time_call(lambda: thetadatadx.to_arrow(ticks))
    print(f"to_arrow:              {t_arrow * 1000:8.1f} ms")

    t_df = time_call(lambda: thetadatadx.to_dataframe(ticks))
    print(f"to_dataframe (pandas): {t_df * 1000:8.1f} ms")

    t_pl = time_call(lambda: thetadatadx.to_polars(ticks))
    print(f"to_polars:             {t_pl * 1000:8.1f} ms")


def run_rss_probe(n: int = 100_000) -> None:
    """RSS-delta probe around a single `to_dataframe` call.

    For a zero-copy Arrow pipeline, RSS growth should approximately
    equal one copy of the buffer set (column-widths * N). A 2x or 3x
    growth would imply the pandas conversion is materializing copies.
    """
    gc.collect()
    ticks = build_eod_ticks(n)
    gc.collect()
    rss_before = rss_kb()
    df = thetadatadx.to_dataframe(ticks)
    gc.collect()
    rss_after = rss_kb()
    delta_bytes = (rss_after - rss_before) * 1024
    # Back-of-envelope: EodTick has 14 primitive cols (~4 i32, 7 f64, 2 i64, 1 str),
    # rough width ~80 bytes/row.
    expected = n * 80
    print(
        f"\n=== RSS probe (N={n:,}) ===\n"
        f"rss_before:   {rss_before:,} KB\n"
        f"rss_after:    {rss_after:,} KB\n"
        f"delta:        {delta_bytes:,} B ({delta_bytes / 1_048_576:.1f} MiB)\n"
        f"expected:     ~{expected:,} B ({expected / 1_048_576:.1f} MiB)\n"
        f"ratio:        {delta_bytes / expected:.2f}x  (zero-copy should be ~1-2x)"
    )
    # Sanity ref to suppress "unused" warnings.
    assert df is not None


def main() -> None:
    for n in (1_000, 10_000, 100_000):
        run_size(n)
    # 1M only if environment is generous.
    if os.environ.get("THETADX_BENCH_BIG") == "1":
        run_size(1_000_000)
    run_rss_probe()


if __name__ == "__main__":
    main()
