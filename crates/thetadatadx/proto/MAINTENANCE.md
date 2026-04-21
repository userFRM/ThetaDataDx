# Proto & Schema Maintenance Guide

This directory contains the protobuf definitions that drive the entire ThetaDataDx SDK.
When you update these files, the build system automatically regenerates gRPC stubs, tick
type structs, and DataTable parsers across all languages.

## Source of truth

**`external.proto` is the canonical wire contract, provided directly by ThetaData engineering.**

**`endpoint_surface.toml` is the canonical endpoint surface contract inside this repository.**
It owns normalized endpoint names, parameter semantics, REST paths, return kinds,
projection call-shapes, reusable parameter groups, and endpoint templates. The
build validates that surface spec against the wire contract in `external.proto`.

Earlier revisions of this crate shipped two separate proto files extracted
from the Java terminal (`endpoints.proto` + `v3_endpoints.proto`). Those have
been superseded by the single-file definition. Do not hand-edit `external.proto`;
request an updated file from ThetaData when the wire protocol changes.

Package: `BetaEndpoints` (everything lives here — shared types, request/response
messages, and the `BetaThetaTerminal` service). Production MDDS routes on this
package name; do not rename without confirming the server has been updated.

## Directory layout

```
proto/
  external.proto    - canonical proto from ThetaData (60 RPCs, BetaEndpoints package)
  MAINTENANCE.md    - this file

../tick_schema.toml    - column schemas for all DataTable-returning endpoints
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
   endpoint registry, shared endpoint runtime dispatch, and `MddsClient`
   endpoint declarations. Outputs: `$OUT_DIR/registry_generated.rs`,
   `$OUT_DIR/endpoint_generated.rs`,
   `$OUT_DIR/mdds_list_endpoints_generated.rs`,
   `$OUT_DIR/mdds_parsed_endpoints_generated.rs`.

3. **Tick parser codegen**: the build reads `tick_schema.toml` and generates
   `DataTable` parser functions. Output: `$OUT_DIR/decode_generated.rs`.
   The public tick structs live in `crates/tdbe/src/types/tick.rs` and must stay
   aligned with that schema.

All three steps are automatic. Just run `cargo build`.

## Endpoint surface spec structure

`endpoint_surface.toml` is intentionally more expressive than the upstream
proto. It supports three layers:

1. **`param_groups.*`** for reusable parameter blocks such as contract
   identity, date ranges, or common builder filters.
2. **`templates.*`** for reusable endpoint families such as stock snapshots or
   option Greeks history. Templates may inherit from each other with `extends`.
3. **`[[endpoints]]`** for concrete endpoint declarations that bind a name,
   description, rest path, return kind, and any endpoint-specific overrides.

The generator expands groups and templates first, then validates the fully
resolved endpoint against `external.proto`. Cycles, unknown references, unused
groups/templates, and invalid overrides fail the build.

## How to: add a new column to an existing endpoint

Example: ThetaData adds a `vwap` column to the EOD response.

1. Open `../tick_schema.toml`
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

If the response uses a new column layout, add a type to `../tick_schema.toml`:
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

Prefer reusing existing `param_groups` and `templates` instead of copying whole
parameter blocks. If the new endpoint introduces a new repeated family shape,
add a new template or parameter group first and then reference it from the
concrete endpoint entry.

**Step 4 — Build and test**

```bash
cargo build        # generates stubs + structs + parser
cargo test         # verify nothing broke
cargo clippy       # zero warnings
```

The new endpoint is now available on `ThetaDataDx` via `Deref` to `MddsClient`.

## How to: replace `external.proto`

When ThetaData ships a new version:

1. Back up the current file: `cp external.proto external.proto.bak`
2. Drop in the new `external.proto`
3. Run `cargo build` — if the proto is valid, stubs regenerate automatically
4. If any RPCs were renamed or removed, `cargo build` will fail validation when
   `endpoint_surface.toml` no longer matches the wire contract. Fix the spec.
5. If new RPCs were added, add corresponding entries to `endpoint_surface.toml`.
6. If column schemas changed, update `tick_schema.toml` to match.
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
reference, or look at the existing entries in `tick_schema.toml` as examples.
