# ThetaDataDx

High-performance Rust SDK for ThetaData market data — single-language core, five language bindings, sub-millisecond decode.

[![build](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg)](https://crates.io/crates/thetadatadx)
[![PyPI](https://img.shields.io/pypi/v/thetadatadx)](https://pypi.org/project/thetadatadx)
[![npm](https://img.shields.io/npm/v/thetadatadx)](https://www.npmjs.com/package/thetadatadx)
[![Docs](https://img.shields.io/docsrs/thetadatadx)](https://docs.rs/thetadatadx)
[![Discord](https://img.shields.io/badge/Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

`thetadatadx` is a native Rust SDK for [ThetaData](https://thetadata.us) market data. It connects directly to ThetaData's MDDS (historical, gRPC) and FPSS (real-time, TCP) services, decodes ticks in-process, and exposes a typed surface across Rust, Python, TypeScript, Go, and C++ from a single Rust core. No JVM, no subprocess, no IPC serialization.

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. The SDK authenticates against ThetaData's Nexus API using your account credentials.

## Highlights

- **Typed everywhere.** 61 ThetaData endpoints exposed as typed methods across all five SDKs; zero raw JSON or protobuf on the public surface.
- **Arrow-backed DataFrames.** Python `to_arrow()` / `to_pandas()` / `to_polars()` pipe through zero-copy Arrow buffers.
- **SPKI-pinned FPSS TLS.** Public-key pinning on the FPSS streaming handshake (stricter than a system-CA trust flow).
- **Sub-millisecond decode.** Nibble-packed FIT decoder and lock-free ring buffer on the streaming path; no JVM warmup, no GC pauses.
- **Zero-copy FFI.** Go, C++, and Node.js go through the same `extern "C"` layer; Python wheel ships via PyO3 ABI3.
- **Feature-complete against the Java terminal.** Same MDDS gRPC contract, same FPSS wire format, same reconnect semantics. See [Java parity checklist](docs/java-parity-checklist.md).

## Quick start

> [!TIP]
> Credentials can be supplied as a `creds.txt` file (email on line 1, password on line 2), inline via `Credentials::new("email", "password")`, or through the `THETADATA_EMAIL` / `THETADATA_PASSWORD` environment variables.

### Rust

```toml
[dependencies]
thetadatadx = "7.3"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={} H={} L={} C={} V={}",
            tick.date, tick.open, tick.high, tick.low, tick.close, tick.volume);
    }
    Ok(())
}
```

### Python

```sh
pip install thetadatadx
```

```python
from thetadatadx import Credentials, Config, ThetaDataDx

tdx = ThetaDataDx(Credentials.from_file("creds.txt"), Config.production())
for tick in tdx.stock_history_eod("AAPL", "20240101", "20240301"):
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")
```

### TypeScript / Node.js

```sh
npm install thetadatadx
```

```typescript
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');
for (const t of tdx.stockHistoryEod('AAPL', '20240101', '20240301')) {
    console.log(`${t.date}: O=${t.open} H=${t.high} L=${t.low} C=${t.close} V=${t.volume}`);
}
```

### Go

```sh
go get github.com/userFRM/ThetaDataDx/sdks/go
```

```go
package main

import (
    "fmt"
    thetadata "github.com/userFRM/ThetaDataDx/sdks/go"
)

func main() {
    tdx, err := thetadata.ConnectFromFile("creds.txt", thetadata.Production())
    if err != nil { panic(err) }
    defer tdx.Close()
    ticks, _ := tdx.StockHistoryEod("AAPL", "20240101", "20240301")
    for _, t := range ticks {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
            t.Date, t.Open, t.High, t.Low, t.Close, t.Volume)
    }
}
```

### C++

```cpp
#include <thetadx.hpp>
#include <cstdio>

int main() {
    auto tdx = thetadatadx::ThetaDataDx::connect_from_file("creds.txt");
    for (const auto& t : tdx.stock_history_eod("AAPL", "20240101", "20240301")) {
        std::printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%lld\n",
            t.date, t.open, t.high, t.low, t.close, (long long)t.volume);
    }
}
```

## Streaming

One connection, one auth. Historical queries are available immediately; streaming connects lazily on first subscription. The client auto-reconnects and re-subscribes all active contracts on involuntary disconnect.

```rust
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
            println!("Quote: {contract_id} bid={bid} ask={ask}");
        }
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
            println!("Trade: {contract_id} @ {price} x {size}");
        }
        FpssEvent::Data(FpssData::Ohlcvc { contract_id, open, high, low, close, volume, .. }) => {
            println!("OHLCVC: {contract_id} O={open} H={high} L={low} C={close} V={volume}");
        }
        _ => {}
    }
})?;

tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
tdx.subscribe_trades(&Contract::stock("AAPL"))?;
```

All prices (`bid`, `ask`, `price`, `open`, `high`, `low`, `close`) are `f64`, decoded during parsing.

## API coverage

61 registry/REST endpoints plus 4 SDK-only historical stream variants, FPSS real-time streaming, and a full Black-Scholes Greeks calculator.

| Category | Endpoints | Examples |
|----------|-----------|----------|
| Stock | 14 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 34 | Same as stock + 5 Greeks tiers, open interest, contracts |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holiday schedule |
| Interest Rate | 1 | EOD rate history |
| Streaming | 7 | Quotes, trades, OI, full-trades (per-contract or firehose) |
| Greeks | 14 | All 22 Greeks + IV solver, individually or batched |

All endpoints return fully typed data in every language. See the [API Reference](docs/api-reference.md) for the complete method list.

## Architecture

```
            +------------------------------------------------+
            |                thetadatadx (Rust)              |
            |                                                |
            |  auth  |  MDDS gRPC  |  FPSS TCP  |  decode    |
            |                                                |
            |                 tdbe (types / codec / Greeks)  |
            +------------------------------------------------+
                                     |
              +---------+-------------+-------------+---------+
              |         |             |             |         |
           PyO3     napi-rs         CGo          C FFI      tonic
             |         |             |             |         |
           Python  TypeScript       Go           C++       Rust
```

| Layer | Crate / package | Purpose |
|-------|-----------------|---------|
| Encoding / types | [`crates/tdbe`](crates/tdbe/) | Tick structs, FIT/FIE codecs, Greeks, Price |
| Core SDK | [`crates/thetadatadx`](crates/thetadatadx/) | MDDS gRPC client, FPSS streaming, auth |
| C FFI | [`ffi/`](ffi/) | Stable `extern "C"` layer consumed by Go, C++, Node.js |
| Python | [`sdks/python`](sdks/python/) | PyO3 / maturin wheel with Arrow DataFrame adapter |
| TypeScript | [`sdks/typescript`](sdks/typescript/) | napi-rs prebuilt binary |
| Go | [`sdks/go`](sdks/go/) | CGo bindings over the FFI layer |
| C++ | [`sdks/cpp`](sdks/cpp/) | RAII header-only wrapper |
| CLI | [`tools/cli`](tools/cli/) | `tdx` CLI — all 61 endpoints from the command line |
| MCP | [`tools/mcp`](tools/mcp/) | MCP server - gives LLMs access to 64 tools over JSON-RPC |
| Server | [`tools/server`](tools/server/) | REST + WebSocket server, feature-compatible with the Java terminal |
| Docs | [`docs/`](docs/) | API reference, architecture, Java parity checklist |
| Website | [`docs-site/`](docs-site/) | VitePress documentation site (deployed to GitHub Pages) |
| Notebooks | [`notebooks/`](notebooks/) | 7 Jupyter notebooks (101-107) |

## Java terminal parity

`thetadatadx` implements the ThetaData MDDS and FPSS wire protocols and covers the same endpoint surface as the Java terminal. Feature-by-feature parity, intentional deviations, and value-adds are tracked in [`docs/java-parity-checklist.md`](docs/java-parity-checklist.md).

## Documentation

| Document | Description |
|----------|-------------|
| [API Reference](docs/api-reference.md) | All 65 methods, 13 tick types, configuration options |
| [Architecture](docs/architecture.md) | System design, wire protocols, TOML codegen pipeline |
| [Java Parity Checklist](docs/java-parity-checklist.md) | Feature-by-feature comparison with the Java terminal |
| [Endpoint Schema](docs/endpoint-schema.md) | TOML codegen format for adding new types/columns |
| [Proto Maintenance](crates/thetadatadx/proto/MAINTENANCE.md) | Guide for updating proto files |

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, pre-commit checks, and pull-request process. Community discussion happens on the [ThetaData Discord](https://discord.thetadata.us/).

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](./LICENSE).
