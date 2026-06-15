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

CI runs `cargo bench` on the tracked benches (mock-based, no
live creds required) and feeds the resulting `target/criterion/`
tree into `scripts/check_bench_regression.py`. The script fails
when any tracked bench's p50 has regressed by more than **25 %**
versus the baseline — the threshold was calibrated against the
GitHub-hosted runner's observed local↔CI spread in PR #566. A 10×
slowdown that previously landed silently now fails the build.

### Deferred entries

The `_deferred` object at the bottom of `criterion.json` lists
benches that should eventually join the gated set but cannot land
their first baseline without a clean GitHub-hosted runner sample.
Each entry's value is the one-line reason and the unblock signal.
The script skips any key starting with `_`, so deferred entries do
not perturb the gate.

### Sub-10 ns p50 baselines

Entries with a `p50_ns` below ~10 ns (e.g. the `StreamEvent::match/*`
family) are noise-dominated under a 25 % proportional threshold:
ordinary timer jitter on a 2-vCPU runner can swing a 1 ns
measurement by ±50 %. The gated set keeps those entries because
they catch order-of-magnitude regressions (a 50 ns p50 against a
1 ns baseline is unambiguous), but a tighter throughput-based
gate is the right answer if these benches ever flap; track that
follow-up in the next bench-tuning PR.

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
