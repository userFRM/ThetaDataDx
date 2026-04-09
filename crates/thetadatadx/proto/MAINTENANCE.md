# Proto & Schema Maintenance Guide

This directory contains the protobuf definitions that drive the entire ThetaDataDx SDK.
When you update these files, the build system automatically regenerates gRPC stubs, tick
type structs, and DataTable parsers across all languages.

## Source of truth

**`external.proto` is the canonical wire contract, provided directly by ThetaData engineering.**

**`endpoint_surface.toml` is the canonical endpoint surface contract inside this repository.**
It owns normalized endpoint names, parameter semantics, REST paths, return kinds,
and projection call-shapes. The build validates that surface spec against the
wire contract in `external.proto`.

We used to maintain reverse-engineered protos (`endpoints.proto` + `v3_endpoints.proto`)
extracted from `ThetaTerminalv3.jar` via `FileDescriptor` reflection. Those have been
replaced by the official single-file definition. Do not reverse-engineer or hand-edit;
request an updated `external.proto` from ThetaData when the wire protocol changes.

Package: `BetaEndpoints` (everything lives here — shared types, request/response
messages, and the `BetaThetaTerminal` service). Production MDDS routes on this
package name; do not rename without confirming the server has been updated.

## Directory layout

```
proto/
  external.proto    - canonical proto from ThetaData (60 RPCs, BetaEndpoints package)
  MAINTENANCE.md    - this file

../endpoint_schema.toml    - column schemas for all DataTable-returning endpoints
../endpoint_surface.toml   - normalized endpoint surface specification
../build.rs                - small build entrypoint
../build_support/          - build-time generators and validators
```

## What happens on `cargo build`

1. **Proto compilation**: `tonic-prost-build` compiles `external.proto` into Rust gRPC
   client stubs and message types. Output: `$OUT_DIR/beta_endpoints.rs`, exposed
   at `crate::proto`.

2. **Endpoint surface validation + generation**: the build loads
   `endpoint_surface.toml`, parses `external.proto` to extract wire metadata,
   validates the surface spec against the wire contract, and generates the
   endpoint registry, shared endpoint runtime dispatch, and `DirectClient`
   endpoint declarations. Outputs: `$OUT_DIR/registry_generated.rs`,
   `$OUT_DIR/endpoint_generated.rs`,
   `$OUT_DIR/direct_list_endpoints_generated.rs`,
   `$OUT_DIR/direct_parsed_endpoints_generated.rs`.

3. **Tick type codegen**: the build reads `endpoint_schema.toml` and generates typed Rust
   structs and DataTable parser functions. Output: `$OUT_DIR/tick_generated.rs`,
   `$OUT_DIR/decode_generated.rs`.

All three steps are automatic. Just run `cargo build`.

## How to: add a new column to an existing endpoint

Example: ThetaData adds a `vwap` column to the EOD response.

1. Open `../endpoint_schema.toml`
2. Find the `[types.EodTick]` section
3. Add one line to the `columns` array:
   ```toml
   { name = "vwap", field = "vwap", type = "price" },
   ```
4. Run `cargo build` — the `EodTick` struct now has a `vwap: f64` field and the
   parser extracts it from the DataTable automatically.

## How to: add a new RPC endpoint

Example: ThetaData adds `GetStockHistoryVwap` to the service.

**Step 1 — Update `external.proto`** (usually via a new file from ThetaData):

```protobuf
message StockHistoryVwapRequestQuery {
  string symbol = 1;
  string start_date = 2;
  string end_date = 3;
  string interval = 4;
}

message StockHistoryVwapRequest {
  QueryInfo query_info = 1;
  StockHistoryVwapRequestQuery params = 2;
}

service BetaThetaTerminal {
  // ... existing RPCs ...
  rpc GetStockHistoryVwap (StockHistoryVwapRequest) returns (stream ResponseData);
}
```

**Step 2 — Column schema**

If the response uses a new column layout, add a type to `../endpoint_schema.toml`:
```toml
[types.VwapTick]
doc = "Volume-weighted average price tick."
copy = true
align = 64
parser = "parse_vwap_ticks"
columns = [
    { name = "ms_of_day", field = "ms_of_day", type = "i32" },
    { name = "vwap",      field = "vwap",      type = "price" },
    { name = "volume",    field = "volume",    type = "i32" },
    { name = "date",      field = "date",      type = "i32" },
]
```

If the response reuses an existing layout (e.g., OHLC bars), skip this step and
use the existing type.

**Step 3 — Add the endpoint surface entry**

Add a new entry to `../endpoint_surface.toml` describing the normalized endpoint
surface. The build will validate it against the wire contract in
`external.proto` and generate the SDK-facing declarations automatically.

**Step 4 — Build and test**

```bash
cargo build        # generates stubs + structs + parser
cargo test         # verify nothing broke
cargo clippy       # zero warnings
```

The new endpoint is now available on `ThetaDataDx` via `Deref` to `DirectClient`.

## How to: replace `external.proto`

When ThetaData ships a new version:

1. Back up the current file: `cp external.proto external.proto.bak`
2. Drop in the new `external.proto`
3. Run `cargo build` — if the proto is valid, stubs regenerate automatically
4. If any RPCs were renamed or removed, `cargo build` will fail validation when
   `endpoint_surface.toml` no longer matches the wire contract. Fix the spec.
5. If new RPCs were added, add corresponding entries to `endpoint_surface.toml`.
6. If column schemas changed, update `endpoint_schema.toml` to match.
7. Run `cargo test` to verify everything works.

Note: the single-file `external.proto` layout means you no longer need to worry
about cross-package references. `ContractSpec`, `QueryInfo`, `DataTable` etc.
are all in the same package as the request/response types.

## Column type reference

| TOML type     | Rust type | What it reads from DataTable cells                    |
|:--------------|:----------|:------------------------------------------------------|
| `i32`         | `i32`     | `Number` cell, cast to i32                            |
| `i64`         | `i64`     | `Number` cell, as i64                                 |
| `f64`         | `f64`     | `Number` cell, as f64 (also Price cells for Greeks)   |
| `String`      | `String`  | `Text` cell                                           |
| `price`       | `f64`     | `Price` cell decoded to `f64` at parse time           |
| `eod_price`   | `f64`     | Either `Price` cell decoded or `Number` cast to f64   |
| `eod_num`     | `i32`     | Either `Price.value` or `Number` (pre-f64 legacy)     |

## Questions?

If anything is unclear, check `docs/endpoint-schema.md` for the full TOML schema
reference, or look at the existing entries in `endpoint_schema.toml` as examples.
