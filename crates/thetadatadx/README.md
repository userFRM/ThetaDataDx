# thetadatadx

Core Rust crate - direct wire-protocol access to ThetaData's MDDS (gRPC) and FPSS (TCP) servers.

This is the engine that powers all ThetaDataDx SDKs (Python, TypeScript/Node.js, Go, C++, CLI, MCP, REST server).

## Entry Point

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

// Historical - 61 typed endpoints, available immediately
let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;

// Streaming - connects lazily on first call
tdx.start_streaming(|event| { /* ... */ })?;
tdx.subscribe_quotes(&Contract::stock("AAPL"))?;

// When done
tdx.stop_streaming();
```

`ThetaDataDx::connect()` authenticates once. Historical data (MDDS gRPC) is available immediately via `Deref` to the internal `MddsClient`. Streaming (FPSS TCP) connects lazily when you call `start_streaming()`.

## Crate Layout

```
src/
  lib.rs           - public re-exports (ThetaDataDx, Credentials, DirectConfig, Error)
  unified.rs       - ThetaDataDx: single entry point, lazy streaming
  mdds/            - MddsClient module (client, stream, validate, normalize, endpoints)
  auth/            - Nexus API authentication, credential parsing
  fpss/            - FPSS streaming client (sync, LMAX Disruptor ring buffer)
  codec/           - FIT nibble encoder/decoder, delta compression
  config.rs        - DirectConfig (server addresses, timeouts, concurrency)
  decode.rs        - DataTable -> typed tick parsing (generated from TOML)
  types/           - Tick structs, Price, enums (generated from TOML)
  greeks.rs        - 22 Black-Scholes Greeks + IV solver
  registry.rs      - Endpoint metadata (generated from the endpoint surface spec)
  error.rs         - Error enum
proto/
  mdds.proto           - canonical MDDS wire contract from ThetaData
  MAINTENANCE.md       - endpoint/proto maintenance guide
tick_schema.toml   - single source of truth for tick type definitions
endpoint_surface.toml  - explicit endpoint surface spec for registry/mdds/runtime generation
build.rs               - small build entrypoint
build_support/         - build-time generators for tick decoding and endpoint surfaces
```

## TOML Codegen

All 13 tick types and their DataTable parsers are generated at compile time from `tick_schema.toml`. Adding a new column is one line in the TOML. See [docs/endpoint-schema.md](../../docs/endpoint-schema.md).

## Endpoint Surface Spec

Endpoint projections are generated from the checked-in `endpoint_surface.toml`
file, which defines the normalized endpoint surface: names, descriptions,
parameter semantics, REST paths, return kinds, projection call-shapes, reusable
parameter groups, and endpoint templates. Templates support inheritance via
`extends`, so the spec can model repeated endpoint families without copying the
same parameter blocks across every declaration.

The build pipeline validates that surface spec against `proto/mdds.proto`
before generating the registry, shared endpoint runtime, and `MddsClient`
endpoint declarations.

## Tick Types

| Type | Fields | Use |
|------|--------|-----|
| `TradeTick` | 16 | Individual trades |
| `QuoteTick` | 11 | NBBO quotes |
| `OhlcTick` | 9 | Aggregated OHLC bars |
| `EodTick` | 18 | End-of-day summary |
| `TradeQuoteTick` | 26 | Combined trade + quote |
| `OpenInterestTick` | 3 | Open interest |
| `MarketValueTick` | 7 | Market cap, shares out, etc. |
| `GreeksTick` | 24 | All 22 Greeks + ms_of_day + date |
| `IvTick` | 4 | Implied volatility + error |
| `PriceTick` | 4 | Index price |
| `CalendarDay` | 5 | Market open/close schedule |
| `InterestRateTick` | 3 | Interest rate |
| `OptionContract` | 5 | Contract definition (root, exp, strike, right) |

All types except `OptionContract` are `Copy` - pure stack values, zero heap allocation.
