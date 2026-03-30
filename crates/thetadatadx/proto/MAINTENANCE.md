# Proto & Schema Maintenance Guide

This directory contains the protobuf definitions that drive the entire ThetaDataDx SDK.
When you update these files, the build system automatically regenerates gRPC stubs, tick
type structs, and DataTable parsers across all languages.

## Directory layout

```
proto/
  endpoints.proto          - shared types (DataTable, ResponseData, Price, etc.)
  v3_endpoints.proto       - v3 service (BetaThetaTerminal, 60 RPCs, QueryInfo wrapper)
  MAINTENANCE.md           - this file

../endpoint_schema.toml    - column schemas for all DataTable-returning endpoints
../build.rs                - reads both proto/ and endpoint_schema.toml, generates everything
```

## What happens on `cargo build`

1. **Proto compilation**: `tonic-prost-build` compiles both `.proto` files into Rust gRPC
   client stubs and message types. Output: `$OUT_DIR/endpoints.rs`, `$OUT_DIR/beta_endpoints.rs`.

2. **Endpoint registry**: `build.rs` parses `v3_endpoints.proto` with regex to extract all
   RPC names, parameter types, and return types. Output: `$OUT_DIR/registry_generated.rs`.

3. **Tick type codegen**: `build.rs` reads `endpoint_schema.toml` and generates typed Rust
   structs and DataTable parser functions. Output: `$OUT_DIR/tick_generated.rs`,
   `$OUT_DIR/decode_generated.rs`.

All three steps are automatic. Just run `cargo build`.

## How to: add a new column to an existing endpoint

Example: ThetaData adds a `vwap` column to the EOD response.

1. Open `../endpoint_schema.toml`
2. Find the `[types.EodTick]` section
3. Add one line to the `columns` array:
   ```toml
   { name = "vwap", field = "vwap", type = "i32" },
   ```
4. Run `cargo build` - the `EodTick` struct now has a `vwap: i32` field and the
   parser extracts it from the DataTable automatically.
5. If the column uses Price encoding, use `type = "price"` or `type = "price_value"` instead.
6. If it's a float, use `type = "f64"`.

## How to: add a new RPC endpoint

Example: ThetaData adds `GetStockHistoryVwap` to the v3 service.

**Step 1 - Proto**

Add the RPC to `v3_endpoints.proto`:
```protobuf
rpc GetStockHistoryVwap(StockHistoryVwapRequest) returns (stream ResponseData);

message StockHistoryVwapRequest {
    QueryInfo query_info = 1;
    StockHistoryVwapParams params = 2;
}

message StockHistoryVwapParams {
    string root = 1;
    string start_date = 2;
    string end_date = 3;
    string interval = 4;
}
```

**Step 2 - Column schema**

If the response uses a new column layout, add a type to `../endpoint_schema.toml`:
```toml
[types.VwapTick]
doc = "Volume-weighted average price tick."
copy = true
align = 64
parser = "parse_vwap_ticks"
columns = [
    { name = "ms_of_day", field = "ms_of_day", type = "i32" },
    { name = "vwap", field = "vwap", type = "price_value" },
    { name = "volume", field = "volume", type = "i32" },
    { name = "price_type", field = "price_type", type = "i32", price_source = "vwap" },
    { name = "date", field = "date", type = "i32" },
]
```

If the response reuses an existing layout (e.g., OHLC bars), skip this step and
use the existing type.

**Step 3 - Wire it up**

In `src/direct.rs`, add:
```rust
parsed_endpoint! {
    /// Fetch VWAP history for a stock.
    fn stock_history_vwap(symbol: &str, start: &str, end: &str, interval: &str) -> Vec<VwapTick>;
    grpc: get_stock_history_vwap;
    request: StockHistoryVwapRequest;
    query: StockHistoryVwapParams {
        root: symbol.to_string(),
        start_date: start.to_string(),
        end_date: end.to_string(),
        interval: interval.to_string(),
    };
    parse: decode::parse_vwap_ticks;
    dates: start, end;
}
```

**Step 4 - Build and test**

```bash
cargo build        # generates stubs + structs + parser
cargo test         # verify nothing broke
cargo clippy       # zero warnings
```

The new endpoint is now available on `ThetaDataDx` via `Deref` to `DirectClient`.

## How to: update the proto files

When replacing the proto files entirely:

1. Back up the current files: `cp endpoints.proto endpoints.proto.bak`
2. Drop in the new proto files
3. Run `cargo build` - if the proto is valid, stubs regenerate automatically
4. If any RPCs were renamed or removed, `cargo build` will fail with compile errors
   pointing to the broken `parsed_endpoint!` calls in `direct.rs`. Fix those.
5. If new RPCs were added, add `parsed_endpoint!` calls (see above).
6. If column schemas changed, update `endpoint_schema.toml` to match.
7. Run `cargo test` to verify everything works.

## Column type reference

| TOML type      | Rust type | What it reads from DataTable cells    |
|:---------------|:----------|:--------------------------------------|
| `i32`          | `i32`     | `Number` cell, cast to i32            |
| `i64`          | `i64`     | `Number` cell, as i64                 |
| `f64`          | `f64`     | `Number` cell, as f64                 |
| `String`       | `String`  | `Text` cell                           |
| `price`        | `i32`     | `Price` cell's `.value`, or `Number`  |
| `price_value`  | `i32`     | Always from `Price` cell's `.value`   |
| `eod_num`      | `i32`     | Either `Price.value` or `Number`      |

## Questions?

If anything is unclear, check `docs/endpoint-schema.md` for the full TOML schema
reference, or look at the existing entries in `endpoint_schema.toml` as examples.
