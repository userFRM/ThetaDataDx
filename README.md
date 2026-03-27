# ThetaDataDx

No-JVM ThetaData Terminal — native Rust SDK for direct market data access.

[![build](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml)
[![Documentation](https://img.shields.io/docsrs/thetadatadx)](https://docs.rs/thetadatadx)
[![license](https://img.shields.io/github/license/userFRM/ThetaDataDx?color=blue)](./LICENSE)
[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg)](https://crates.io/crates/thetadatadx)
[![PyPI](https://img.shields.io/pypi/v/thetadatadx)](https://pypi.org/project/thetadatadx)
[![Discord](https://img.shields.io/badge/join_Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

## Overview

`thetadatadx` connects directly to ThetaData's upstream servers — MDDS for historical data and FPSS for real-time streaming — entirely in native Rust. No JVM terminal process, no local Java dependency, no subprocess management. Your application talks to ThetaData's infrastructure with the same wire protocol their own terminal uses.

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. This SDK authenticates against ThetaData's Nexus API using your account credentials.

## Features

- **Historical data** via MDDS/gRPC — EOD, OHLC, trades, quotes across stocks, options, and indices
- **Real-time streaming** via FPSS/TCP — live quotes, trades, open interest, and OHLC snapshots
- **Full Greeks calculator** — 22 Black-Scholes Greeks (first, second, and third order) plus IV solver
- **Zero-copy tick types** — `TradeTick`, `QuoteTick`, `OhlcTick`, `EodTick` with fixed-point `Price` encoding
- **Async/await** throughout — built on Tokio with concurrent gRPC streaming and background heartbeat tasks
- **Direct authentication** — handles the Nexus API auth flow, session management, and reconnection logic
- **FIT codec** — native decoder for ThetaData's nibble-encoded delta-compressed tick format
- **Multi-language SDKs** — Python (PyO3), Go (CGo), C++ (RAII), all powered by the Rust core, all with FPSS streaming support
- **pandas DataFrame support** — `to_dataframe()` and `_df` convenience methods in the Python SDK

## Installation

### Rust

```toml
[dependencies]
thetadatadx = "3.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

### Python

```sh
pip install thetadatadx

# With pandas DataFrame support
pip install thetadatadx[pandas]
```

## Quick Start

> [!TIP]
> Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2. This is the same format the Java terminal uses.

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    // Fetch end-of-day stock data
    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={} H={} L={} C={} V={}",
            tick.date, tick.open_price(), tick.high_price(),
            tick.low_price(), tick.close_price(), tick.volume);
    }

    // List option expirations
    let exps = tdx.option_list_expirations("SPY").await?;
    println!("SPY expirations: {:?}", &exps[..5.min(exps.len())]);

    Ok(())
}
```

## Streaming Example

> [!NOTE]
> FPSS streaming connects to ThetaData's dedicated streaming servers via TLS/TCP. The client automatically sends heartbeat pings every 100ms as required by the protocol.

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    // Start streaming with a callback
    tdx.start_streaming(|event: &FpssEvent| {
        match event {
            FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
                println!("Quote: contract={contract_id} bid={bid} ask={ask}");
            }
            FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
                println!("Trade: contract={contract_id} price={price} size={size}");
            }
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                println!("Contract {id} = {contract}");
            }
            _ => {}
        }
    })?;

    tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
    println!("Subscribed to AAPL quotes");

    // Block until shutdown
    std::thread::park();
    tdx.stop_streaming();
    Ok(())
}
```

## Supported Endpoints

61 typed methods covering all ThetaData MDDS endpoints (60 gRPC RPCs).

### Stock (13 methods)

| Category | Method | Description |
|----------|--------|-------------|
| List | `stock_list_symbols()` | All available stock symbols |
| List | `stock_list_dates()` | Available dates for a stock by request type |
| Snapshot | `stock_snapshot_ohlc()` | Latest OHLC snapshot |
| Snapshot | `stock_snapshot_trade()` | Latest trade snapshot |
| Snapshot | `stock_snapshot_quote()` | Latest NBBO quote snapshot |
| Snapshot | `stock_snapshot_market_value()` | Latest market value snapshot |
| History | `stock_history_eod()` | End-of-day data for a date range |
| History | `stock_history_ohlc()` | Intraday OHLC bars for a single date |
| History | `stock_history_ohlc_range()` | Intraday OHLC bars across a date range |
| History | `stock_history_trade()` | All trades on a given date |
| History | `stock_history_quote()` | NBBO quotes at a given interval |
| History | `stock_history_trade_quote()` | Combined trade + quote ticks |
| AtTime | `stock_at_time_trade()` | Trade at a specific time across a date range |
| AtTime | `stock_at_time_quote()` | Quote at a specific time across a date range |

### Option (34 methods)

| Category | Method | Description |
|----------|--------|-------------|
| List | `option_list_symbols()` | All available option underlyings |
| List | `option_list_dates()` | Available dates for an option contract |
| List | `option_list_expirations()` | Expiration dates for an underlying |
| List | `option_list_strikes()` | Strike prices for a given expiration |
| List | `option_list_contracts()` | All contracts for a symbol on a date |
| Snapshot | `option_snapshot_ohlc()` | Latest OHLC snapshot for options |
| Snapshot | `option_snapshot_trade()` | Latest trade snapshot for options |
| Snapshot | `option_snapshot_quote()` | Latest NBBO quote snapshot for options |
| Snapshot | `option_snapshot_open_interest()` | Latest open interest snapshot |
| Snapshot | `option_snapshot_market_value()` | Latest market value snapshot |
| Snapshot Greeks | `option_snapshot_greeks_implied_volatility()` | IV snapshot |
| Snapshot Greeks | `option_snapshot_greeks_all()` | All Greeks snapshot |
| Snapshot Greeks | `option_snapshot_greeks_first_order()` | First-order Greeks snapshot |
| Snapshot Greeks | `option_snapshot_greeks_second_order()` | Second-order Greeks snapshot |
| Snapshot Greeks | `option_snapshot_greeks_third_order()` | Third-order Greeks snapshot |
| History | `option_history_eod()` | End-of-day option data |
| History | `option_history_ohlc()` | Intraday option OHLC bars |
| History | `option_history_trade()` | Option trades on a given date |
| History | `option_history_quote()` | Option NBBO quotes |
| History | `option_history_trade_quote()` | Combined trade + quote ticks |
| History | `option_history_open_interest()` | Open interest history |
| History Greeks | `option_history_greeks_eod()` | EOD Greeks history |
| History Greeks | `option_history_greeks_all()` | All Greeks history (intraday) |
| History Greeks | `option_history_greeks_first_order()` | First-order Greeks history |
| History Greeks | `option_history_greeks_second_order()` | Second-order Greeks history |
| History Greeks | `option_history_greeks_third_order()` | Third-order Greeks history |
| History Greeks | `option_history_greeks_implied_volatility()` | IV history (intraday) |
| History Trade Greeks | `option_history_trade_greeks_all()` | All Greeks per trade |
| History Trade Greeks | `option_history_trade_greeks_first_order()` | First-order Greeks per trade |
| History Trade Greeks | `option_history_trade_greeks_second_order()` | Second-order Greeks per trade |
| History Trade Greeks | `option_history_trade_greeks_third_order()` | Third-order Greeks per trade |
| History Trade Greeks | `option_history_trade_greeks_implied_volatility()` | IV per trade |
| AtTime | `option_at_time_trade()` | Trade at a specific time across a date range |
| AtTime | `option_at_time_quote()` | Quote at a specific time across a date range |

### Index (9 methods)

| Category | Method | Description |
|----------|--------|-------------|
| List | `index_list_symbols()` | All available index symbols |
| List | `index_list_dates()` | Available dates for an index |
| Snapshot | `index_snapshot_ohlc()` | Latest OHLC snapshot |
| Snapshot | `index_snapshot_price()` | Latest price snapshot |
| Snapshot | `index_snapshot_market_value()` | Latest market value snapshot |
| History | `index_history_eod()` | End-of-day index data |
| History | `index_history_ohlc()` | Intraday OHLC bars |
| History | `index_history_price()` | Intraday price history |
| AtTime | `index_at_time_price()` | Price at a specific time across a date range |

### Interest Rate (1 method)

| Category | Method | Description |
|----------|--------|-------------|
| History | `interest_rate_history_eod()` | End-of-day interest rate history |

### Calendar (3 methods)

| Category | Method | Description |
|----------|--------|-------------|
| Calendar | `calendar_open_today()` | Whether the market is open today |
| Calendar | `calendar_on_date()` | Calendar info for a specific date |
| Calendar | `calendar_year()` | Calendar info for an entire year |

### Streaming (FPSS)

| Method | Description |
|--------|-------------|
| `subscribe_quotes()` | Real-time quote stream for a contract |
| `subscribe_trades()` | Real-time trade stream for a contract |
| `subscribe_open_interest()` | Real-time open interest for a contract |
| `subscribe_full_trades()` | All trades for a security type |
| `unsubscribe_quotes()` | Stop quote stream |
| `unsubscribe_trades()` | Stop trade stream |
| `unsubscribe_open_interest()` | Stop open interest stream |

### Greeks Calculator

| Function | Description |
|----------|-------------|
| `greeks::all_greeks()` | Compute all 22 Greeks + IV in one call |
| `greeks::implied_volatility()` | IV solver via bisection |
| `greeks::delta()` | First-order: delta |
| `greeks::gamma()` | Second-order: gamma |
| `greeks::theta()` | First-order: theta (daily) |
| `greeks::vega()` | First-order: vega |
| `greeks::rho()` | First-order: rho |
| `greeks::vanna()` | Second-order: vanna |
| `greeks::charm()` | Second-order: charm |
| `greeks::vomma()` | Second-order: vomma |
| `greeks::speed()` | Third-order: speed |
| `greeks::zomma()` | Third-order: zomma |
| `greeks::color()` | Third-order: color |
| `greeks::ultima()` | Third-order: ultima |

## Configuration

```rust
use thetadatadx::{ThetaDataDx, DirectConfig};

// Production (ThetaData NJ datacenter, gRPC over TLS)
let config = DirectConfig::production();

// Dev (same servers, shorter timeouts for faster iteration)
let config = DirectConfig::dev();

// Custom configuration (override specific fields)
let config = DirectConfig {
    fpss_timeout_ms: 5_000,
    reconnect_wait_ms: 2_000,
    ..DirectConfig::production()
};

// Override gRPC concurrency (default is auto-detected from subscription tier)
let config = DirectConfig {
    mdds_concurrent_requests: Some(8),  // manual override; None = auto (2^tier)
    ..DirectConfig::production()
};
```

> [!TIP]
> Credentials can be loaded from a file, environment variables, or constructed directly:
> ```rust
> let creds = Credentials::from_file("creds.txt")?;
> let creds = Credentials::new(std::env::var("THETA_EMAIL")?, std::env::var("THETA_PASS")?);
> ```

## Documentation

- **[API Reference](docs/api-reference.md)** — All client methods, tick types, enums, and configuration options
- **[Architecture](docs/architecture.md)** — System design, protocol specifications, wire formats, and auth flow
- **[JVM Deviations](docs/jvm-deviations.md)** — Intentional differences from the Java terminal
- **[Reverse-Engineering Guide](docs/reverse-engineering.md)** — How to decompile the terminal JAR and extract protocol definitions

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## Disclaimer

> [!CAUTION]
> Theta Data, ThetaData, and Theta Terminal are trademarks of Theta Data, Inc. / AxiomX LLC. This project is **not affiliated with, endorsed by, or supported by Theta Data**.

ThetaDataDx is an independent, open-source project provided "as is", without warranty of any kind.

### How ThetaDataDx Was Built

ThetaDataDx was developed through independent analysis of the ThetaData Terminal JAR and its network protocol. The protocol implementation was built from scratch in Rust based on decompiled Java source and observed wire-level behavior. This approach is consistent with the principle of interoperability through protocol analysis — the same method used by projects like Samba (SMB/CIFS), open-source Exchange clients, and countless other third-party implementations of proprietary network protocols.

### Legal Considerations

> [!WARNING]
> - **No warranty.** ThetaDataDx is provided "as is", without warranty of any kind. See [LICENSE](./LICENSE) for full terms.
> - **Use at your own risk.** Users are solely responsible for ensuring their use complies with ThetaData's Terms of Service and any applicable laws or regulations. Using ThetaDataDx may carry risks including but not limited to account restriction or termination.
> - **Not financial software.** ThetaDataDx is a research and interoperability project. It is not intended as a replacement for officially supported ThetaData software in production trading environments. The authors accept no liability for financial losses, missed trades, or any other damages arising from the use of this software.
> - **Protocol stability.** ThetaDataDx relies on an undocumented protocol that ThetaData may change at any time without notice. There is no guarantee of continued functionality.

### EU Interoperability

For users and contributors in the European Union: Article 6 of the EU Software Directive (2009/24/EC) permits reverse engineering for the purpose of achieving interoperability with independently created software, provided that specific conditions are met. ThetaDataDx was developed with this legal framework in mind, enabling interoperability with ThetaData's market data infrastructure on platforms where the official Java-based Terminal cannot run (headless Linux, containers, embedded systems, native Rust/Go/C++ applications).

## License

GPL-3.0-or-later — see [LICENSE](./LICENSE).
