# ThetaDataDx

No-JVM ThetaData Terminal - native Rust SDK for direct market data access.

[![build](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml)
[![Documentation](https://img.shields.io/docsrs/thetadatadx)](https://docs.rs/thetadatadx)
[![license](https://img.shields.io/github/license/userFRM/ThetaDataDx?color=blue)](./LICENSE)
[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg)](https://crates.io/crates/thetadatadx)
[![PyPI](https://img.shields.io/pypi/v/thetadatadx)](https://pypi.org/project/thetadatadx)
[![Discord](https://img.shields.io/badge/join_Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

## Overview

`thetadatadx` connects directly to ThetaData's upstream servers - MDDS for historical data and FPSS for real-time streaming - entirely in native Rust. No JVM terminal process, no local Java dependency, no subprocess management. Your application talks to ThetaData's infrastructure with the same wire protocol their own terminal uses.

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. This SDK authenticates against ThetaData's Nexus API using your account credentials.

## Repository Structure

| Path | Description |
|------|-------------|
| [`crates/tdbe/`](crates/tdbe/) | ThetaData Binary Encoding - types, FIT/FIE codecs, Greeks, Price |
| [`crates/thetadatadx/`](crates/thetadatadx/) | Core Rust SDK - gRPC historical, FPSS streaming (depends on `tdbe`) |
| [`sdks/python/`](sdks/python/) | Python SDK (PyO3/maturin) - `pip install thetadatadx` |
| [`sdks/go/`](sdks/go/) | Go SDK (CGo FFI) |
| [`sdks/cpp/`](sdks/cpp/) | C++ SDK (RAII wrappers over C FFI) |
| [`ffi/`](ffi/) | C FFI layer - shared library consumed by Go and C++ |
| [`tools/cli/`](tools/cli/) | `tdx` CLI - all 61 endpoints from the command line |
| [`tools/mcp/`](tools/mcp/) | MCP server - gives LLMs access to 64 tools over JSON-RPC |
| [`tools/server/`](tools/server/) | REST+WS server - drop-in replacement for the Java terminal |
| [`docs/`](docs/) | Architecture, API reference, JVM deviations, reverse-engineering guide |
| [`docs-site/`](docs-site/) | mdBook documentation site (deployed to GitHub Pages) |
| [`notebooks/`](notebooks/) | 7 Jupyter notebooks (101-107) |

## Quick Start

> [!TIP]
> Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2. This is the same format the Java terminal uses.

### Rust

```toml
[dependencies]
thetadatadx = "3.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    // Or inline: let creds = Credentials::new("user@example.com", "your-password");
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={} H={} L={} C={} V={}",
            tick.date, tick.open_price(), tick.high_price(),
            tick.low_price(), tick.close_price(), tick.volume);
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

creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDx(creds, Config.production())

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} H={tick['high']:.2f} "
          f"L={tick['low']:.2f} C={tick['close']:.2f} V={tick['volume']}")
```

## Streaming

One connection, one auth. Historical available immediately, streaming connects lazily.

```rust
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
            println!("Trade: {contract_id} @ {price} x {size}");
        }
        _ => {}
    }
})?;

tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
```

## API Coverage

61 typed endpoints covering all ThetaData MDDS data, plus FPSS real-time streaming and a full Black-Scholes Greeks calculator.

| Category | Endpoints | Examples |
|----------|-----------|---------|
| Stock | 14 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 34 | Same as stock + 5 Greeks tiers, open interest, contracts |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holiday schedule |
| Interest Rate | 1 | EOD rate history |
| Streaming | 7 | Quotes, trades, OI, full-trades (per-contract or firehose) |
| Greeks | 14 | All 22 Greeks + IV solver, individually or batched |

All endpoints return fully typed native structs in every language. Zero raw JSON or protobuf in the public API. See the [API Reference](docs/api-reference.md) for the complete method list.

## Documentation

| Document | Description |
|----------|-------------|
| [API Reference](docs/api-reference.md) | All 61 methods, 14 tick types, configuration options |
| [Architecture](docs/architecture.md) | System design, wire protocols, TOML codegen pipeline |
| [JVM Deviations](docs/jvm-deviations.md) | Intentional differences from the Java terminal |
| [Reverse-Engineering Guide](docs/reverse-engineering.md) | How to decompile the terminal and extract proto definitions |
| [Endpoint Schema](docs/endpoint-schema.md) | TOML codegen format for adding new types/columns |
| [Proto Maintenance](crates/thetadatadx/proto/MAINTENANCE.md) | Guide for ThetaData engineers updating proto files |

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## Disclaimer

> [!CAUTION]
> Theta Data, ThetaData, and Theta Terminal are trademarks of Theta Data, Inc. / AxiomX LLC. This project is **not affiliated with, endorsed by, or supported by Theta Data**.

ThetaDataDx is an independent, open-source project provided "as is", without warranty of any kind.

### How ThetaDataDx Was Built

ThetaDataDx was developed through independent analysis of the ThetaData Terminal JAR and its network protocol. The protocol implementation was built from scratch in Rust based on decompiled Java source and observed wire-level behavior. This approach is consistent with the principle of interoperability through protocol analysis - the same method used by projects like Samba (SMB/CIFS), open-source Exchange clients, and countless other third-party implementations of proprietary network protocols.

### Legal Considerations

> [!WARNING]
> - **No warranty.** ThetaDataDx is provided "as is", without warranty of any kind. See [LICENSE](./LICENSE) for full terms.
> - **Use at your own risk.** Users are solely responsible for ensuring their use complies with ThetaData's Terms of Service and any applicable laws or regulations. Using ThetaDataDx may carry risks including but not limited to account restriction or termination.
> - **Not financial software.** ThetaDataDx is a research and interoperability project. It is not intended as a replacement for officially supported ThetaData software in production trading environments. The authors accept no liability for financial losses, missed trades, or any other damages arising from the use of this software.
> - **Protocol stability.** ThetaDataDx relies on an undocumented protocol that ThetaData may change at any time without notice. There is no guarantee of continued functionality.

### EU Interoperability

For users and contributors in the European Union: Article 6 of the EU Software Directive (2009/24/EC) permits reverse engineering for the purpose of achieving interoperability with independently created software, provided that specific conditions are met. ThetaDataDx was developed with this legal framework in mind, enabling interoperability with ThetaData's market data infrastructure on platforms where the official Java-based Terminal cannot run (headless Linux, containers, embedded systems, native Rust/Go/C++ applications).

## License

GPL-3.0-or-later - see [LICENSE](./LICENSE).
