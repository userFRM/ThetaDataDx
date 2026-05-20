# Criterion Baseline

This directory holds the committed bench-regression baseline consumed
by the CI `bench` job.

`criterion.json` lists the headline benches the gate watches, each
entry mapping a stable display ID to:

* `p50_ns` — the last-blessed median runtime in nanoseconds.
* `criterion_path` — the relative path under `target/criterion/`
  where Criterion writes that bench's `new/estimates.json`. Criterion
  replaces `/` in `criterion_group()` names with `_`; the bench
  function name keeps its real form. Both are reflected here.

## How the gate runs

CI runs `cargo bench` on the four tracked benches (mock-based, no
live creds required) and feeds the resulting `target/criterion/`
tree into `scripts/check_bench_regression.py`. The script fails
when any tracked bench's p50 has regressed by more than 20% versus
the baseline. A 10× slowdown that previously landed silently now
fails the build.

## How to refresh the baseline

The baseline is **not** auto-rebased: a benign noise spike or an
intentional perf regression both need a human in the loop.

1. Land any genuine perf-affecting changes on `main`.
2. On a clean checkout of green `main`, run:
   ```
   cargo bench -p thetadatadx \
       --bench grpc_channel -- "stock_list_symbols/in_house" --quick
   cargo bench -p thetadatadx \
       --bench streaming_channels -- "streaming_channels/" --quick
   ```
3. Read the freshly written `target/criterion/<...>/new/estimates.json`
   files; copy each `median.point_estimate` into `criterion.json`
   here.
4. Open a separate PR that **only** updates `criterion.json`. The PR
   description should name the underlying change that caused the
   shift and the percent delta it produced.

Refresh PRs are review-gated specifically because they unblock
otherwise-failing perf gates — they're the single chokepoint that
catches a maintainer ratifying a regression by accident.
