# Proto & Schema Maintenance Guide

This directory contains the protobuf definitions that drive the entire Client SDK.
When you update these files, the build system automatically regenerates gRPC stubs, tick
type structs, and DataTable parsers across all languages.

## Source of truth

**`mdds.proto` is the canonical wire contract, provided directly by ThetaData engineering.**

**The endpoint table in `../src/surface/endpoints.rs` is the endpoint surface of every binding.** It owns normalized endpoint names, parameter semantics, REST paths, return kinds and documentation. Every entry builds its request from the message types generated from `mdds.proto`, so an entry that does not match the wire contract does not compile. `generate_sdk_surfaces` renders the Python type stub, the C and C++ headers, the live validators and the docs site's endpoint pages from the same table, and fails while an RPC of `mdds.proto` has no entry.

Earlier revisions of this crate shipped two separate proto files
(`endpoints.proto` + `v3_endpoints.proto`). Those have
been superseded by the single-file definition. Do not hand-edit `mdds.proto`;
request an updated file from ThetaData when the wire protocol changes.

Package: `BetaEndpoints` (everything lives here: shared types, request/response
messages, and the `BetaThetaTerminal` service). Production MDDS routes on this
package name; do not rename without confirming the server has been updated.

## Directory layout

```
proto/
  mdds.proto    - canonical proto from ThetaData (60 RPCs, BetaEndpoints package)
  MAINTENANCE.md    - this file

../src/surface/ticks.rs     - tick table (every tick type the endpoints return)
../src/surface/endpoints.rs - endpoint table (every endpoint surface)
../build.rs                - small build entrypoint
../build_support/          - build-time generators and validators
```

## What happens on `cargo build`

The gRPC client stubs and message types are **not** compiled on a normal
build. They are pre-generated and committed at
`proto/beta_endpoints.snapshot.rs` (included directly by `src/lib.rs` and
exposed at `crate::proto`), so a default `cargo build` needs no `protoc`.
Regenerating them from `mdds.proto` happens only under the `grpc-codegen`
feature; see "replace `mdds.proto`" below.

Nothing is generated on a default build. The endpoint table in `src/surface/endpoints.rs` expands at compile time into the endpoint registry, the shared endpoint dispatch, and the `MarketDataClient` methods and builders, and the tick table in `src/surface/ticks.rs` into the tick structs, their `DataTable` decoders and their Arrow and Polars builders.

The build needs no `protoc`. Under the `grpc-codegen`
feature the build additionally recompiles `mdds.proto` with `protoc` and
drift-checks the committed snapshot; CI runs that check.

## Endpoint table structure

The table has two layers:

1. **`__param!`** defines each parameter once (its wire name, whether it is
   required, its type, its default and its description) and names reusable
   groups of parameters, such as contract identity or date ranges.
2. **`__endpoints!`** declares each endpoint: its shape, name, group, REST
   path, return type, the parameter keys it takes and its documentation.

The module documentation of `../src/surface/endpoints.rs` describes the
grammar. An unknown key or a malformed entry does not expand, and an entry
whose parameters do not match its request message does not compile.
`generate_sdk_surfaces` rejects what the compiler lets through: an unknown
shape, a list entry that does not return `StringList` or a non-list entry
that does, a default its type does not accept, and an enum parameter whose
description does not list exactly the values of its enum.

## How to: add a new column to an existing endpoint

Example: ThetaData adds a `vwap` column to the EOD response.

1. Open `../src/surface/ticks.rs`
2. Find the `EodTick` entry
3. Add one line to its `columns` list, and raise the field count in its first `doc` line, which the compiler checks:
   ```rust
   vwap: price "Volume-weighted average price.",
   ```
4. Run `cargo build`. The `EodTick` struct now has a `vwap: f64` field, the decoder extracts it from the DataTable, and the Python and TypeScript row classes carry it.

## How to: add a new RPC endpoint

Example: ThetaData adds `GetStockHistoryVwap` to the service.

**Step 1: Update `mdds.proto`** (usually via a new file from ThetaData):

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

**Step 2: Tick type**

If the response uses a new column layout, add an entry to the tick table in `../src/surface/ticks.rs`. Its module documentation describes the entry grammar:
```rust
VwapTick {
    doc: ["Volume-weighted average price tick" -- 4 fields. "One bar of the VWAP series."],
    required: [ms_of_day vwap],
    columns: [
        ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
        vwap: price "Volume-weighted average price.",
        volume: i32 "Shares traded in the bar.",
        date: i32 "Trading date as a YYYYMMDD integer.",
    ],
    timestamps: [timestamp_ms(date, ms_of_day)],
    flags: [],
}
```

If the response reuses an existing layout (e.g., OHLC bars), skip this step and
use the existing type.

**Step 3: Add the endpoint surface entry**

Add a new entry to the endpoint table in `../src/surface/endpoints.rs`. Its module documentation describes the entry grammar, and the compiler checks the entry against the request messages.

Prefer reusing the parameter definitions and groups in `__param!` instead of
defining a parameter again. If the new endpoint introduces a new repeated
family shape, add a group first and then reference it from the entry.
`generate_sdk_surfaces` renders the binding surfaces and the docs pages from
the entry.

**Step 4: Build and test**

```bash
# If mdds.proto changed, regenerate the committed gRPC snapshot first
# (needs protoc) and commit the diff:
cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot --features grpc-codegen
cargo build        # expands the endpoint and tick tables (uses the committed snapshot)
cargo run -p thetadatadx-rs --features config-file --bin generate_sdk_surfaces  # binding surfaces
cargo test         # verify nothing broke
cargo clippy       # zero warnings
```

The new endpoint is now available on `Client` via `Deref` to `MarketDataClient`.

## How to: replace `mdds.proto`

When ThetaData ships a new version:

1. Back up the current file: `cp mdds.proto mdds.proto.bak`
2. Drop in the new `mdds.proto`
3. Regenerate + commit the gRPC snapshot (this is the only step that needs
   `protoc`): `cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot --features grpc-codegen`,
   then commit the `proto/beta_endpoints.snapshot.rs` diff. CI drift-checks
   this under `--features grpc-codegen`, so a stale snapshot fails the build.
4. If any RPCs were renamed or removed, the endpoint table entries that no longer match the wire contract fail to compile. Fix them.
5. If new RPCs were added, add corresponding entries to the endpoint table; `generate_sdk_surfaces` fails until every RPC has one.
6. If column layouts changed, update the tick table (`../src/surface/ticks.rs`) to match.
7. Run `cargo test` to verify everything works.

Note: the single-file `mdds.proto` layout means you no longer need to worry
about cross-package references. `ContractSpec`, `QueryInfo`, `DataTable` etc.
are all in the same package as the request/response types.

## Column type reference

| Column type   | Rust type | What it reads from DataTable cells                    |
|:--------------|:----------|:------------------------------------------------------|
| `i32`         | `i32`     | `Number` cell, cast to i32                            |
| `i64`         | `i64`     | `Number` cell, as i64                                 |
| `f64`         | `f64`     | `Number` cell, as f64 (also Price cells for Greeks)   |
| `String`      | `String`  | `Text` cell                                           |
| `price`       | `f64`     | `Price` cell decoded to `f64` at parse time           |
| `eod_price`   | `f64`     | Either `Price` cell decoded or `Number` cast to f64   |
| `eod_num`     | `i32`     | Either `Price.value` or `Number` (pre-f64 legacy)     |

## Questions?

If anything is unclear, the module documentation of `../src/surface/ticks.rs` and `../src/surface/endpoints.rs` describes each table's grammar, and the existing entries are examples.
