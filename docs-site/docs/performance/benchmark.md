---
title: Benchmark
description: Full benchmark matrix — ThetaDataDx vs the ThetaData Python SDK across every entitled endpoint. Reproduction commands and known regressions.
---

# Benchmark

An independent harness compared ThetaDataDx against the ThetaData Python SDK (`thetadata==1.0.1`). The harness is in a separate repo, `userFRM/thetadata-bench-v2`; results below are paraphrased from its `REVIEW.md`.

## Setup

| Axis | Value |
|------|-------|
| Machine | Intel i7-10700KF (8 cores / 16 threads), Linux 6.8, 64 GB RAM |
| Python | 3.14.4 |
| Libraries compared | `thetadata==1.0.1` vs `thetadatadx 8.0.1` |
| Subscription | STANDARD stock / PROFESSIONAL option (index + interest rate excluded) |
| Measurement | Best-of-N wall-clock across 3 measured reps + 1 warmup, per subprocess |
| Memory | Peak `VmHWM` delta from `/proc/self/status`, subprocess-isolated |
| Matrix | 50 endpoints × 3 libraries × 3 reps = 442 successful measurements |

Three library variants: the ThetaData Python SDK (`thetadata`), ThetaDataDx with the default `list[TickClass]` return (`dx_pyclass`), and ThetaDataDx with `to_arrow(ticks)` (`dx_arrow`). Every measurement isolated to a fresh subprocess so the peak RSS read is clean.

## Headline — top 10 wins

| Endpoint | Rows | Python SDK wall | ThetaDataDx wall | Speedup | Python SDK RSS | ThetaDataDx RSS | RSS ratio |
|----------|-----:|----------------:|-----------------:|--------:|---------------:|----------------:|----------:|
| `option_history_greeks_all` | 176,732 | 61.94 s | 11.06 s | **5.60×** | 730.8 MB | 61.4 MB | 11.9× |
| `option_history_greeks_second_order` | 176,732 | 37.47 s | 6.83 s | **5.49×** | 389.8 MB | 81.1 MB | 4.8× |
| `option_history_greeks_first_order` | 176,732 | 41.47 s | 7.78 s | **5.33×** | 403.4 MB | 58.4 MB | 6.9× |
| `option_history_ohlc` | 117,691 | 16.02 s | 3.15 s | **5.08×** | 162.8 MB | 42.8 MB | 3.8× |
| `option_history_greeks_iv` | 176,732 | 42.29 s | 9.33 s | **4.53×** | 342.1 MB | 41.2 MB | 8.3× |
| `option_history_greeks_third_order` | 176,732 | 34.83 s | 8.46 s | **4.12×** | 363.7 MB | 88.1 MB | 4.1× |
| `option_history_quote` | 176,732 | 26.02 s | 6.46 s | **4.03×** | 212.6 MB | 44.6 MB | 4.8× |
| `option_snapshot_greeks_all` | 498 | 0.364 s | 0.130 s | **2.80×** | 2.2 MB | 1.9 MB | 1.2× |
| `option_history_trade` | 47,050 | 7.56 s | 3.32 s | **2.28×** | 51.5 MB | 17.1 MB | 3.0× |
| `stock_list_symbols` | 25,498 | 0.768 s | 0.355 s | **2.16×** | 4.0 MB | 1.7 MB | 2.4× |

`ThetaDataDx wall` / `ThetaDataDx RSS` columns use the `to_arrow` variant. The pyclass-only variant is often faster on RSS (up to **75× less RAM** on `option_history_greeks_first_order`) at a small wall-time cost.

## Memory — peak RSS delta

| Endpoint | Python SDK | ThetaDataDx arrow | ThetaDataDx pyclass | Best ratio |
|----------|-----------:|------------------:|--------------------:|-----------:|
| `option_history_greeks_all` | 730.8 MB | 61.4 MB | 19.0 MB | **38× less** |
| `option_history_greeks_first_order` | 403.4 MB | 58.4 MB | 5.5 MB | **73× less** |
| `option_history_greeks_second_order` | 389.8 MB | 81.1 MB | 5.2 MB | **75× less** |
| `option_history_greeks_third_order` | 363.7 MB | 88.1 MB | 5.1 MB | **71× less** |
| `option_history_greeks_iv` | 342.1 MB | 41.2 MB | 21.3 MB | **16× less** |
| `option_history_ohlc` | 162.8 MB | 42.8 MB | 19.4 MB | **8× less** |
| `option_history_quote` | 212.6 MB | 44.6 MB | 16.0 MB | **13× less** |
| `stock_history_trade` (955,237 rows) | 426.0 MB | 97.6 MB | 36.9 MB | **12× less** |

The arrow-path RSS runs higher than the pyclass-path because the current implementation holds both the pyclass list and the Arrow buffers alive at peak. Endpoint-direct Arrow variants — skip the pyclass list entirely — are tracked as a follow-up; they would bring the arrow-path RSS to pyclass-path levels or below.

## Where ThetaDataDx ties {#ties}

Small snapshot / calendar / list-of-dates calls (≤100 rows) are within ±5 % of the Python SDK. The measurement floor is network RTT; both libraries hit the same wire:

| Endpoint | Rows | Python SDK | ThetaDataDx | Ratio |
|----------|-----:|-----------:|------------:|------:|
| `calendar_open_today` | 1 | 98 ms | 92 ms | 1.06× |
| `option_at_time_trade` | 789 | 7.34 s | 7.09 s | 1.04× |
| `stock_snapshot_ohlc` | 10 | 98 ms | 97 ms | 1.01× |
| `calendar_year` | 13 | 93 ms | 93 ms | 1.00× |
| `stock_list_dates` | 2,343 | 140 ms | 139 ms | 1.00× |
| `option_list_strikes` | 232 | 99 ms | 100 ms | 0.99× |
| `option_list_expirations` | 2,026 | 133 ms | 141 ms | 0.94× |

No win, no loss. Pick either library on small payloads.

## Where ThetaDataDx loses {#losses}

Two cells where ThetaDataDx runs measurably slower than the Python SDK:

| Endpoint | Rows | Python SDK | ThetaDataDx | Ratio | Note |
|----------|-----:|-----------:|------------:|------:|------|
| `option_history_open_interest` | 452 | 2.73 s | 3.16 s | 0.866× | `dx_pyclass` variant wins at 1.99 s — arrow conversion overhead dominates here |
| `stock_at_time_trade` | 21 | 7.26 s | 11.59 s | **0.626×** | 21-row result taking ~11 s — network-bound for all three libs; ThetaDataDx spends ~4 s somewhere in the async path |

Both are tracked investigations, not hidden regressions. The `stock_at_time_trade` slowdown on a 21-row result is the single clearest win-back target in the matrix.

## Concurrency {#concurrency}

The benchmark harness includes a `harness/concurrency.py` tool that runs any scenario at arbitrary worker counts via `ThreadPoolExecutor` and reports p50 / p95 / throughput. A concurrency sweep was **not executed** as part of the v2 run — this is the single most credibility-adding follow-up for a future pass.

Expected shape, based on the Rust core's architecture:

- At `workers ≤ tier_cap` (STANDARD: 4 stock, PRO: 8 option), ThetaDataDx should scale near-linearly. The tokio runtime handles fan-out natively, gRPC multiplexes on one HTTP/2 channel, and decode runs off-GIL on Rust threads.
- The Python SDK under `ThreadPoolExecutor` bottlenecks on the GIL because its materialize loop is pure-Python; aggregate throughput improves less than linearly.

Mark any concurrency claim as projected until a workers=1..8 sweep is attached to the matrix.

## Scale curves

The bench repo has `charts/scale_latency.png` and `charts/scale_rss.png` covering three endpoints at five row-count tiers (`scale` plan). The v2 generation of the charts produced empty axes — the harness had a data-ingest gap at chart-render time. Treat them as missing artifacts until the v3 rerun. The full per-cell data is still in `results/raw.jsonl`.

## Stage breakdown

The single most informative measurement in the bench is the per-stage split of the Python SDK's `_convert_response_stream`:

| Scenario | Drain | Materialize | Convert | Wall |
|----------|------:|------------:|--------:|-----:|
| `option_history_greeks_all` (176k × 31) | 7.78 s | 47.29 s | 4.66 s | 61.94 s |
| `option_history_greeks_first_order` (176k × 17) | 7.31 s | 28.62 s | 4.55 s | 41.47 s |
| `option_history_greeks_iv` (176k × 14) | 12.20 s | 24.79 s | 4.60 s | 42.29 s |
| `stock_history_trade_quote` (955k × 19) | 6.13 s | 137.39 s | 27.18 s | 172.46 s |

Materialize — the `HasField` + `defaultdict(list)` loop in the Python SDK's `client.py` — dominates 70–80 % of wall time on every dense endpoint. This is the bottleneck the Rust decoder replaces.

## Known functional issue

`stock_history_trade_quote` and `option_history_trade_quote` currently return empty lists from ThetaDataDx even though the network round-trip completes (19 s / 3.7 s of wall-time matches a full payload being pulled). The Python SDK returns 955,237 rows and 47,050 rows on the same params. This is a decoder bug, not a perf issue — tracked as P11 in the external review and scheduled as a fix before the bench numbers get quoted in release notes.

## Reproduction

```bash
git clone https://github.com/userFRM/thetadata-bench-v2
cd thetadata-bench-v2

# Full coverage (50 entitled endpoints × 3 libs × 3 reps, ~45 min)
python3 run_bench.py --plan coverage --repeats 3 --fresh

# Scale curves (latency + RSS vs row count) — 3 endpoints × 5 sizes
python3 run_bench.py --plan scale --repeats 3

# Decode-only replay (Python SDK — ThetaDataDx needs a decode_response_bytes FFI hook)
python3 run_bench.py --plan decode --repeats 3

# Concurrency (single scenario, multiple workers)
venvs/venv-dx/bin/python -m harness.concurrency \
    dx_arrow option_history_greeks_all__std 4 3

# Regenerate report from existing raw.jsonl
python3 -c "from harness.report import render; from pathlib import Path; \
    render(Path('results'), Path('BENCHMARK.md'), Path('charts'))"
```

## Artifacts

- `results/raw.jsonl` — 442 measurement JSON lines
- `results/env.json` — machine + library versions for the run
- `results/captures/` — pickled `ResponseData` chunk bytes for decode replay
- `BENCHMARK.md` — machine-generated full tables

## Next

- [Performance summary](../getting-started/performance) — headline table at a glance
- [Migration](../migration/from-thetadata-python-sdk) — call mapping from the Python SDK to ThetaDataDx
