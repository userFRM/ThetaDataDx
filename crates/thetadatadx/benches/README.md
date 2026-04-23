# Benchmarks

43 benchmarks covering every module in thetadatadx, measured with [Criterion.rs](https://github.com/bheisler/criterion.rs).

## Hardware

| Component | Spec |
|-----------|------|
| CPU | Intel Core i7-10700KF @ 3.80GHz (8C/16T, boost 5.1GHz) |
| L1d / L1i | 256 KiB each (8 instances) |
| L2 | 2 MiB (8 instances) |
| L3 | 16 MiB |
| Memory | 128 GB DDR4 |
| OS | Ubuntu 24.04.4 LTS (kernel 6.8.0) |
| Rust | stable 1.94.0, release profile |

## Results

### FIT Codec (`codec/fit.rs`) - 4 benchmarks

| Benchmark | Median | Per-unit | Description |
|-----------|--------|----------|-------------|
| `fit_decode_single_row` | 38.2 ns | 38.2 ns/row | Single trade tick row decode |
| `fit_decode_100_rows` | 4.51 us | 45.1 ns/row | 100 trade tick rows |
| `fit_decode_1000_rows_scalar` | 45.5 us | 45.5 ns/row | 1000 rows, scalar path |
| `fit_delta_decompression` | 3.95 ns | - | apply_deltas on 16-field tick |

### FIE Encoder (`codec/fie.rs`) - 4 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `fie_encode` | 29.5 ns | Encode 10-char string to nibble bytes |
| `fie_try_encode` | 28.2 ns | Same with Result error path |
| `fie_encode_long` | 44.8 ns | Encode 50-char string |
| `fie_decode` | 34.4 ns | Decode nibble bytes back to string |

### Price (`types/price.rs`) - 5 benchmarks

| Benchmark | Median | Per-unit | Description |
|-----------|--------|----------|-------------|
| `price_new_1000` | 641 ns | 0.64 ns/op | Construct Price (validated) |
| `price_to_f64_1000` | 1.02 us | 1.02 ns/op | Convert to f64 via lookup table |
| `price_compare_same_type_1000` | 1.91 us | 1.91 ns/op | Same-type comparison (fast path) |
| `price_compare_1000` | 2.34 us | 2.34 ns/op | Cross-type comparison (scaling) |
| `price_display_1000` | 104.8 us | 104.8 ns/op | Format to string |

### Greeks (`greeks.rs`) - 5 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `greeks_delta_only` | 22.8 ns | Single delta computation |
| `greeks_value` | 37.6 ns | Black-Scholes option value |
| `all_greeks_individual` | 434.6 ns | 22 Greeks via individual function calls |
| `greeks_iv_solver` | 523.2 ns | Implied volatility bisection solver |
| `all_greeks` | 639.9 ns | Full 22 Greeks + IV (precomputed intermediates) |

### Response Decoding (`decode.rs`) - 9 benchmarks

| Benchmark | Median | Per-unit | Description |
|-----------|--------|----------|-------------|
| `decode_zstd_small` | 524 ns | - | Decompress 1KB zstd payload |
| `decode_zstd_large` | 26.9 us | - | Decompress 100KB zstd payload |
| `decode_data_table_10_rows` | 5.24 us | 524 ns/row | Parse protobuf DataTable (10 rows) |
| `decode_data_table_1000_rows` | 395.6 us | 396 ns/row | Parse protobuf DataTable (1000 rows) |
| `decode_extract_number_column` | 948 ns | - | Extract number column from 10-row table |
| `decode_extract_price_column` | 1.30 us | - | Extract price column from 10-row table |
| `parse_trade_ticks_100` | 2.63 us | 26.3 ns/tick | Parse 100 trade ticks from DataTable |
| `parse_quote_ticks_100` | 1.82 us | 18.2 ns/tick | Parse 100 quote ticks |
| `parse_ohlc_ticks_100` | 1.57 us | 15.7 ns/tick | Parse 100 OHLC ticks |

### FPSS Framing (`fpss/framing.rs`) - 3 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `frame_write` | 13.1 ns | Write a 50-byte frame |
| `frame_read` | 43.1 ns | Read a 50-byte frame |
| `frame_roundtrip` | 57.6 ns | Write + read |

### FPSS Protocol (`fpss/protocol.rs`) - 6 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `contract_stock_to_bytes` | 14.9 ns | Serialize stock contract |
| `contract_option_to_bytes` | 15.8 ns | Serialize option contract |
| `contract_from_bytes` | 22.2 ns | Deserialize contract |
| `contract_roundtrip` | 39.9 ns | Serialize + deserialize |
| `build_credentials_payload` | 15.6 ns | Build auth payload |
| `build_subscribe_payload` | 28.3 ns | Build subscription payload |

### Auth (`auth/creds.rs`) - 2 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `creds_parse` | 78.7 ns | Parse creds.txt string |
| `creds_new` | 54.0 ns | Construct credentials |

### Enum Lookups (`types/enums.rs`) - 2 benchmarks

| Benchmark | Median | Per-unit | Description |
|-----------|--------|----------|-------------|
| `stream_msg_type_from_code_1000` | 408 ns | 0.41 ns/op | StreamMsgType::from_code |
| `data_type_from_code_1000` | 2.18 us | 2.18 ns/op | DataType::from_code |

### Tick Operations (`types/tick.rs`) - 3 benchmarks

| Benchmark | Median | Description |
|-----------|--------|-------------|
| `trade_tick_get_price` | 885 ps | TradeTick::get_price() |
| `quote_tick_midpoint` | 2.03 ns | QuoteTick::midpoint_value() |
| `ohlc_tick_all_prices` | 2.51 ns | All 4 OHLC price conversions |

## Key Takeaways

- **Tick field access is sub-nanosecond** (885 ps) thanks to `#[repr(C, align(64))]` cache-line alignment and `#[inline]`
- **FIT decoding is 45 ns/row** (scalar) - the FPSS streaming hot path
- **Delta decompression is 3.95 ns** per 16-field tick - essentially free
- **Price conversion is 1 ns** via static lookup table (no `pow()` call)
- **Full 22-Greek computation is 640 ns** including IV bisection solver. Single delta is 23 ns
- **Trade tick parsing from DataTable is 26 ns/tick** - the entire protobuf-to-typed-struct pipeline
- **FPSS frame I/O is 13-43 ns** per frame - wire layer adds negligible overhead
- **Contract serialization is 15 ns** - subscription messages are built faster than a cache miss

## Running

```bash
# Run all 43 benchmarks
cargo bench -p thetadatadx --bench bench

# Run a specific group
cargo bench -p thetadatadx --bench bench -- fit
cargo bench -p thetadatadx --bench bench -- greeks
cargo bench -p thetadatadx --bench bench -- price
cargo bench -p thetadatadx --bench bench -- decode
cargo bench -p thetadatadx --bench bench -- frame
cargo bench -p thetadatadx --bench bench -- protocol
cargo bench -p thetadatadx --bench bench -- auth
cargo bench -p thetadatadx --bench bench -- enum
cargo bench -p thetadatadx --bench bench -- tick

# Compare against a baseline
cargo bench -p thetadatadx --bench bench -- --save-baseline before
# ... make changes ...
cargo bench -p thetadatadx --bench bench -- --baseline before
```

Criterion writes HTML reports to `target/criterion/`. Open `report/index.html` for interactive charts.
