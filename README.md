# ThetaDataDx

Market data SDKs for [ThetaData](https://thetadata.us) — Rust, Python,
TypeScript, and C++, all powered by one Rust core. Historical queries,
real-time streaming, and bulk flat-file downloads through a single
authenticated client. No JVM, no local terminal.

[![Rust CI](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml)
[![Python SDK](https://github.com/userFRM/ThetaDataDx/actions/workflows/python.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/python.yml)
[![TypeScript SDK](https://github.com/userFRM/ThetaDataDx/actions/workflows/typescript.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/typescript.yml)
[![Deploy Docs](https://github.com/userFRM/ThetaDataDx/actions/workflows/docs.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/docs.yml)

[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg?logo=rust)](https://crates.io/crates/thetadatadx)
[![PyPI](https://img.shields.io/pypi/v/thetadatadx?logo=python&logoColor=white)](https://pypi.org/project/thetadatadx)
[![npm](https://img.shields.io/npm/v/thetadatadx?logo=npm)](https://www.npmjs.com/package/thetadatadx)
[![docs.rs](https://img.shields.io/docsrs/thetadatadx?logo=docsdotrs)](https://docs.rs/thetadatadx)

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Python](https://img.shields.io/badge/python-3.12%2B-blue.svg?logo=python&logoColor=white)](https://www.python.org)
[![Node](https://img.shields.io/badge/node-20%2B-339933.svg?logo=node.js&logoColor=white)](https://nodejs.org)
[![C++](https://img.shields.io/badge/C%2B%2B-17-00599C.svg?logo=cplusplus&logoColor=white)](https://isocpp.org)
[![Discord](https://img.shields.io/badge/Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required.
> The SDK authenticates against ThetaData's Nexus API using your account
> credentials.

## Requirements

- Rust 1.88 or newer (declared on every workspace `[package]`; CI lint
  pinned to this floor).
- A valid [ThetaData](https://thetadata.us) subscription for the live
  endpoints.

## Quick start

> [!TIP]
> Credentials can be supplied as a `creds.txt` file (email on line 1,
> password on line 2), inline via `Credentials::new("email", "password")`,
> or through the `THETADATA_EMAIL` / `THETADATA_PASSWORD` environment
> variables.

### Rust — option Greeks for backtesting

EOD Greeks for an option contract across a date range. Decode to a
typed `Vec<GreeksEodTick>` ready for risk attribution or scenario
analysis.

```toml
[dependencies]
thetadatadx = "12"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    // SPY 2026-06-19 expiration, EOD Greeks over Q1.
    let chain = tdx
        .option_history_greeks_eod("SPY", "20260619", "20240101", "20240331")
        .await?;

    for t in chain.iter().take(5) {
        println!(
            "{} K={:>6.2} {} delta={:+.4} gamma={:+.4} theta={:+.4} vega={:+.4} IV={:.4}",
            t.date, t.strike, t.right, t.delta, t.gamma, t.theta, t.vega, t.implied_vol,
        );
    }
    Ok(())
}
```

Opt into chainable DataFrame ergonomics with the `polars` and/or
`arrow` features (both stay out of the default dep graph):

```toml
thetadatadx = { version = "12", features = ["polars"] }
```

```rust
use thetadatadx::frames::TicksPolarsExt;

let df = chain.as_slice().to_polars()?;
df.lazy().filter(col("delta").gt(lit(0.4))).collect()?;
```

### Python — chain Greeks to a DataFrame

The Python binding emits typed tick wrappers with `.to_polars()` /
`.to_pandas()` / `.to_arrow()` chained from any historical query — no
intermediate row-by-row iteration.

```sh
pip install thetadatadx
```

```python
from thetadatadx import Credentials, Config, ThetaDataDxClient

tdx = ThetaDataDxClient(Credentials.from_file("creds.txt"), Config.production())

# First-order Greeks for one expiration on one date — typically 200-500 rows
# spanning every strike + right pair the chain offers.
ticks = tdx.option_history_greeks_first_order("SPY", "20260619", "20240315")

df = ticks.to_polars()
print(df.select(["strike", "right", "delta", "gamma", "theta", "vega"]).head())
```

### TypeScript / Node.js — live options streaming

The TS binding registers a push callback through `startStreaming(callback)`.
napi-rs `ThreadsafeFunction` routes every event onto the Node main
thread; the TLS reader never touches V8. Compose the typed `Contract`
+ `Subscription` values directly.

```sh
npm install thetadatadx
```

```typescript
import { Credentials, Config, Contract, ThetaDataDxClient } from 'thetadatadx';

const client = new ThetaDataDxClient(
  Credentials.fromFile('creds.txt'),
  Config.production(),
);

client.startStreaming((event) => {
  if (event.kind === 'trade') {
    const { contract, price, size } = event.trade;
    console.log(`${contract.symbol} ${contract.strike}${contract.right} @ ${price} x ${size}`);
  }
});

client.subscribeMany([
  Contract.option('SPY', '20260619', '550', 'C').quote(),
  Contract.option('SPY', '20260619', '550', 'C').trade(),
]);
```

### C++ — low-latency historical decode

The C++ surface is a RAII header-only wrapper over the C ABI. Decoded
tick spans are owned by the response object; no copy on iteration.

```cpp
#include <thetadx.hpp>
#include <cstdio>

int main() {
    auto client = tdx::Client::connect(
        tdx::Credentials::from_file("creds.txt"),
        tdx::Config::production());

    // SPY 550 call on 2026-06-19, intraday Greeks for 2024-03-15.
    auto chain = client.option_history_greeks_first_order(
        "SPY", "20260619", "20240315");

    for (const auto& t : chain) {
        std::printf("%d K=%.2f %c delta=%+.4f gamma=%+.4f theta=%+.4f\n",
                    t.ms_of_day, t.strike, t.right,
                    t.delta, t.gamma, t.theta);
    }
}
```

## Streaming

One connection, one auth. Historical queries are available immediately;
the streaming transport connects lazily on first subscription. The
client auto-reconnects and re-subscribes all active contracts on
involuntary disconnect.

The primary streaming surface is the fluent contract-first API.
`Contract::stock("AAPL").quote()` returns a typed `Subscription` value
that the polymorphic `client.subscribe(...)` accepts directly:

```rust
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::prelude::*;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
            println!("Quote: {} bid={bid} ask={ask}", contract.symbol);
        }
        FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
            println!("Trade: {} @ {price} x {size}", contract.symbol);
        }
        _ => {}
    }
})?;

let stock  = Contract::stock("AAPL");
let option = Contract::option("SPY", "20260620", "550", "C")?;
tdx.subscribe(stock.quote())?;
tdx.subscribe(option.trade())?;
```

For streaming-only workloads, build an `FpssClient` directly and
iterate events on the caller's own thread:

```rust
use thetadatadx::auth::Credentials;
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::{FpssClient, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let hosts = DirectConfig::production().fpss.hosts;
let client = FpssClient::builder(&creds, &hosts)
    .ring_size(8192)
    .build()?;

client.subscribe(Contract::stock("AAPL").quote())?;

for event in &client {
    match event? {
        FpssEvent::Data(data)       => { /* … */ }
        FpssEvent::Control(control) => { /* … */ }
    }
}
```

`next_event` blocks until the next event arrives or the stream ends.
`try_next_event` is the non-blocking cousin. `poll_batch(FnMut)`
and `for_each(FnMut)` are available for the closure-driven shapes.

### Buffered vs streaming for historical pulls

Every historical builder (`option_history_*`, `stock_history_*`,
`index_history_*`, `interest_rate_history_*`) supports two terminals:

| Workload | Terminal |
|---|---|
| Single day / one-shot ad-hoc query | `.await` |
| Bulk / multi-day backfill | `.stream(handler)` |
| Tick-interval responses | `.stream(handler)` |
| Greeks responses across a long horizon | `.stream(handler)` |

Buffered `.await` collects the full response into `Vec<Tick>` before
returning. On a 2.4 M-tick day this consumes ~5 GiB of RSS before any
caller code runs. `.stream(handler)` yields chunks via
`handler(&[Tick])` and drops each chunk before the next is fetched —
peak RSS stays at ~150 MiB regardless of response size. The buffered
path emits a single `tracing::warn!` when the estimated response size
crosses the configured threshold (default 100 MiB; set to `0` to
disable).

## Endpoint coverage

61 typed endpoints across stock, option, index, calendar, and interest
rate surfaces, plus real-time streaming and a local Black-Scholes
Greeks calculator.

| Category | Endpoints | Examples |
|---|---|---|
| Stock | 14 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 34 | Stock surfaces plus five Greeks tiers, open interest, contracts |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holidays, early closes |
| Interest rate | 1 | EOD rate history |

The full method list across all four languages lives in the
[API Reference](https://userfrm.github.io/ThetaDataDx/reference/).

Beyond historical queries: real-time streaming
(subscribe and unsubscribe per contract and per full-stream type)
plus a local Greeks calculator (22 Black-Scholes Greeks plus an IV
solver, callable individually or batched).

## Repository layout

| Path | Package | Purpose |
|---|---|---|
| [`crates/thetadatadx`](crates/thetadatadx/) | `thetadatadx` (crates.io) | The Rust SDK |
| [`crates/tdbe`](crates/tdbe/) | `tdbe` | Shared tick types, Greeks, and price math |
| [`sdks/python`](sdks/python/) | `thetadatadx` (PyPI) | Python package with DataFrame adapters |
| [`sdks/typescript`](sdks/typescript/) | `thetadatadx` (npm) | TypeScript / Node.js package, prebuilt binaries |
| [`sdks/cpp`](sdks/cpp/) | header + prebuilt library | C++ wrapper over the C ABI |
| [`ffi/`](ffi/) | release artifacts | C ABI for embedders |
| [`tools/cli`](tools/cli/) | `tdx` | Command-line client |
| [`tools/server`](tools/server/) | `thetadatadx-server` | Local HTTP / WebSocket server (drop-in terminal replacement) |
| [`tools/mcp`](tools/mcp/) | `thetadatadx-mcp` | MCP server exposing every historical endpoint to AI clients |
| [`docs-site`](docs-site/) | — | Documentation site (GitHub Pages) |

## Documentation

- [Documentation site](https://userfrm.github.io/ThetaDataDx/) — getting started, API reference, streaming, server, MCP
- [Changelog](CHANGELOG.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup,
pre-commit checks, and pull-request process. Community discussion
happens on the [ThetaData Discord](https://discord.thetadata.us/).

## License

Licensed under the Apache License, Version 2.0. See
[LICENSE](./LICENSE).
