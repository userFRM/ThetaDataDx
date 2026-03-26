# Benchmarks

Performance benchmarks for thetadatadx core operations, measured with [Criterion.rs](https://github.com/bheisler/criterion.rs).

## Hardware

- **CPU**: Intel Core i7-10700KF @ 3.80 GHz (8 cores / 16 threads, boost to 5.1 GHz)
- **L1d / L1i**: 256 KiB each (8 instances)
- **L2**: 2 MiB (8 instances)
- **L3**: 16 MiB
- **Memory**: 128 GB DDR4
- **OS**: Ubuntu 24.04.4 LTS (kernel 6.8.0)
- **Rust**: stable 1.85+, release profile with LTO

## Results

| Benchmark | Median | Description |
|---|---|---|
| `fit_decode_100_rows` | **4.64 us** | Decode 100 FIT trade tick rows (scalar path) |
| `fit_decode_1000_rows_scalar` | **46.7 us** | Decode 1000 FIT rows, scalar nibble extraction |
| `fit_decode_1000_rows_simd_bulk` | **100.9 us** | Decode 1000 FIT rows, SSE2 bulk nibble scan |
| `price_to_f64_1000` | **1.02 us** | Convert 1000 `Price` structs to f64 (lookup table) |
| `price_compare_1000` | **2.39 us** | Compare 1000 `Price` pairs (cross-type, lookup table) |
| `all_greeks` | **647 ns** | Full 22 Greeks + IV solver (precomputed intermediates) |
| `all_greeks_individual` | **433 ns** | Same Greeks via individual function calls |
| `fie_encode` | **29.6 ns** | FIE nibble-encode a 10-char string |
| `fie_try_encode` | **28.3 ns** | FIE encode with error handling (`Result` path) |

## Key Takeaways

- **FIT decoding** -- ~46 ns per tick row on the scalar path. The SIMD bulk path (`decode_fit_buffer_bulk`) currently shows higher total latency because it pays a one-time setup cost for the SSE2 scan pass. It is designed for sustained throughput on large buffers (tens of thousands of rows), not per-row latency on small batches.

- **Price operations** -- ~1 ns per f64 conversion, ~2.4 ns per cross-type comparison. A precomputed lookup table eliminates `pow()` calls entirely.

- **Greeks** -- The full 22-Greek computation, including IV bisection solver, completes in under 650 ns. The precomputed-intermediate path (`all_greeks`) is slightly slower than calling each Greek individually (`all_greeks_individual`) because it unconditionally computes every Greek even when only a subset is needed.

- **FIE encoding** -- ~29 ns per string, dominated by the nibble lookup rather than allocation.

## Running Benchmarks

Run all benchmarks in the `thetadatadx` crate:

```bash
cargo bench -p thetadatadx
```

Run a single benchmark by name:

```bash
cargo bench -p thetadatadx -- fit_decode_100_rows
```

### Comparing Against a Baseline

Save a baseline before making changes, then compare after:

```bash
# Record the baseline
cargo bench -p thetadatadx -- --save-baseline before

# ... make your changes ...

# Compare against the saved baseline
cargo bench -p thetadatadx -- --baseline before
```

Criterion writes HTML reports to `target/criterion/`. Open the top-level `report/index.html` in a browser for interactive charts with confidence intervals.

### Environment Tips

- Close other workloads during benchmarking -- background processes add noise.
- Pin the CPU governor to `performance` if you want stable numbers:
  ```bash
  sudo cpupower frequency-set -g performance
  ```
- The benchmarks use synthetic FIT buffers (see `build_fit_buffer` in `bench.rs`), not live data. Results measure codec and math performance in isolation.

## Benchmark Source

All benchmarks live in [`bench.rs`](bench.rs). The Criterion group registers nine functions:

| Function | What It Measures |
|---|---|
| `bench_fit_decode_100_rows` | `FitReader::read_changes` + `apply_deltas` loop, 100 rows |
| `bench_fit_decode_1000_rows_scalar` | Same loop, 1000 rows |
| `bench_fit_decode_1000_rows_simd` | `decode_fit_buffer_bulk` (SSE2 path), 1000 rows |
| `bench_price_to_f64_1000` | `Price::to_f64` with lookup table, 1000 iterations |
| `bench_price_compare_1000` | `Price` cross-type `>` comparison, 1000 pairs |
| `bench_all_greeks` | `greeks::all_greeks` (precomputed intermediates) |
| `bench_all_greeks_individual` | 18 individual Greek function calls |
| `bench_fie_encode` | `string_to_fie_line` (infallible path) |
| `bench_fie_try_encode` | `try_string_to_fie_line` (`Result` path) |
