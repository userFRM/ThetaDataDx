---
title: Performance
description: ThetaDataDx vs the ThetaData Python SDK — benchmark headline with link to the full matrix.
---

# Performance

An independent benchmark harness compared ThetaDataDx against the ThetaData Python SDK (`thetadata==1.0.1`) across every entitled endpoint on a STANDARD stock / PROFESSIONAL option subscription. 442 measurements, three reps each, wall-clock best-of-N and peak `VmHWM` tracked per subprocess.

## Headline

| Axis | Delta (ThetaDataDx vs ThetaData Python SDK) |
|------|---------------------------------------------|
| Wall time, bulk endpoints | up to **9.00×** faster (`stock_history_trade_quote`, 955,237 rows) |
| Wall time, dense Greeks | **5.60×** faster (`option_history_greeks_all`, 176,732 rows × 31 cols) |
| Peak RSS, dense Greeks | **75×** less (`option_history_greeks_first_order`, 176,732 rows × 17 cols) |
| Small-payload calls (≤100 rows) | ±5 % — both libraries hit the same network RTT floor |

Full per-endpoint matrix, memory breakdown, stage-resolved profiling of the decode loop, reproduction commands, and known regressions are on the [benchmark page](../performance/benchmark).

## Where the speedup comes from

The ThetaData Python SDK decode loop is pure-Python: it walks the protobuf `ResponseData` list, calls `HasField(...)` on every tick cell, branches to a per-field `defaultdict(list)` build, then hands the dict to `polars.DataFrame(...)` which allocates its own Arrow buffers. That loop accounts for roughly 70–80 % of wall time on every dense endpoint (see benchmark Section 3). ThetaDataDx replaces it with a Rust decoder that walks the `DataTable` row-major and writes directly into typed slices, which the Python binding then exposes either as `list[TickClass]` or as an `arrow::RecordBatch`.

The 75× RSS win comes from the same place: the Python decode holds millions of `int` / `float` / `str` objects in the `defaultdict(list)` before the polars materialize step copies them into Arrow; the Rust decode writes to a `Vec<StructOfArrays>` directly.

## Concurrency note

The benchmark harness includes a `harness/concurrency.py` tool that runs a scenario at N parallel workers, but a concurrency sweep was not part of the v2 run. The external review documents the expected shape:

- At `workers ≤ tier_cap` (4 stock / 8 option), ThetaDataDx should scale near-linearly — the async runtime handles the fan-out natively and decode is off-GIL.
- The ThetaData Python SDK under `ThreadPoolExecutor` will bottleneck on the GIL because the materialize loop is pure-Python; aggregate throughput improves less than linearly.

Concrete workers=1..8 numbers are a future benchmark pass, not a measured claim. See the [benchmark page Concurrency section](../performance/benchmark#concurrency).

## Where ThetaDataDx ties or loses

The benchmark is explicit about every cell where ThetaDataDx does not win:

- Small snapshot / calendar / list-of-dates calls (≤100 rows) are within ±5 % of the Python SDK. The measurement floor is network RTT and both libraries hit the same wire.
- `option_history_greeks_eod` (2,200 rows) has a known `dx_pyclass` regression; the `to_arrow` path still wins.
- `stock_at_time_trade` (21 rows) is the one endpoint where ThetaDataDx is measurably slower than the ThetaData Python SDK; investigation tracked on the benchmark page.

See the [benchmark page](../performance/benchmark#losses) for the full breakdown.

## Next

- [Benchmark page](../performance/benchmark) — full matrix, reproduction, regressions
- [Async Python](./async-python) — concurrency patterns at subscription caps
- [Migration](./migration) — one-to-one call mapping from the Python SDK
