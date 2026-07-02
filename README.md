<p align="center">
  <img src="assets/logo.svg" alt="ThetaDataDx" width="100%" />
</p>

# ThetaDataDx

High-performance market-data SDKs for [ThetaData](https://thetadata.us), in **Python, TypeScript, C++, and Rust**. One Rust engine under all four. Pull US stock, option, index, and rate data three ways: point-in-time **history**, real-time **streaming**, and whole-universe **flat files**, all from a single authenticated client. Connects straight to ThetaData, with nothing to install and run locally.

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
> A valid [ThetaData](https://thetadata.us) subscription is required. The SDK
> authenticates against ThetaData's Nexus API using your account credentials.

## Features

- **Complete coverage**: stocks, options, indices, and rates across 65 typed endpoints.
- **Three access modes, one client**: point-in-time history, real-time streaming, and bulk flat-file downloads.
- **DataFrames built in**: every result chains straight to Polars, pandas, or Arrow over a zero-copy boundary.
- **Greeks on demand**: first- through third-order Greeks and implied volatility, served straight from the option endpoints.
- **The same surface in every language**: identical methods and identical typed errors, Python through Rust.
- **No terminal to run**: a direct connection to ThetaData; nothing to install and babysit locally.

## Install

```bash
pip install thetadatadx        # Python
npm install thetadatadx        # TypeScript / Node.js
cargo add thetadatadx          # Rust
```

Point an AI client (Claude Desktop, Cursor, and others) at the MCP server, no install and no Rust toolchain:

```json
{ "command": "npx", "args": ["-y", "thetadatadx-mcp"], "env": { "THETADATA_API_KEY": "your_key" } }
```

C++ ships as a header plus a small implementation file over a prebuilt library (a CMake target wires it up). See the [C++ guide](thetadatadx-cpp/).

## Quick start

> [!TIP]
> Pass your API key directly to the client and you are one line from a live connection. Generate a key from your [ThetaData user portal](https://www.thetadata.net/), then hand it to the client: `Client(api_key="td1_...")` in Python, `Client.connectWith({ apiKey: "td1_..." })` in TypeScript, `Client::builder().api_key("td1_...").connect()` in Rust and C++. The key can come from `THETADATA_API_KEY` (the env source) or a `.env` file instead of an inline literal. Email and password is also supported: pass `email` and `password` inline, load a `creds.txt` file (email on line 1, password on line 2), or read the `THETADATA_EMAIL` / `THETADATA_PASSWORD` environment variables. For full control over hosts and timeouts, build a typed `Credentials` + `Config` (see each SDK's "full control" example).

### Python

```python
from thetadatadx import Client

# Pass your API key directly. Use historical_type="STAGE" to target staging.
client = Client(api_key="td1_...")

# First-order Greeks for every strike on SPY's 2026-06-19 expiry, as of 2024-03-15
greeks = client.historical.option_history_greeks_first_order("SPY", "20260619", "20240315")

df = greeks.to_polars()
print(df.select(["strike", "right", "delta", "gamma", "theta", "vega"]).head())
```

Other ways to construct the client:

```python
from thetadatadx import Client, Credentials, Config

# API key from the THETADATA_API_KEY environment variable, or from a .env file
client = Client.from_env()
client = Client.from_dotenv(".env")

# Email and password, inline
client = Client(email="you@example.com", password="your_password")

# Full control: build a typed Credentials + Config (custom hosts, timeouts)
client = Client(Credentials.from_file("creds.txt"), Config.production())
```

Stream live quotes and trades through the same client. The callback matches on
typed event classes:

```python
import time
from thetadatadx import Contract, MarketValue, Quote, Trade

def on_event(event):
    match event:
        case Trade(price=px, size=sz, exchange=ex, ms_of_day=ms, sequence=seq, condition=cond, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} trade price={px:.2f} size={sz} "
                f"exchange={ex} ms_of_day={ms} sequence={seq} condition={cond}"
            )
        case Quote(bid=b, ask=a, bid_size=bs, ask_size=asz, bid_exchange=bx, ask_exchange=ax, ms_of_day=ms, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} quote bid={b:.2f} ask={a:.2f} "
                f"bid_size={bs} ask_size={asz} bid_exchange={bx} "
                f"ask_exchange={ax} ms_of_day={ms}"
            )
        case MarketValue(market_bid=mb, market_ask=ma, market_price=mp, ms_of_day=ms, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} market_value "
                f"bid={mb:.2f} ask={ma:.2f} price={mp:.2f} ms_of_day={ms}"
            )

spy_call = Contract.option("SPY", expiration="20260619", strike="550", right="C")

with client.streaming(on_event) as session:
    session.subscribe_many([spy_call.quote(), spy_call.trade(), spy_call.market_value()])
    time.sleep(60)   # park the main thread while events flow into on_event
```

### TypeScript

```typescript
import { Contract, Client } from 'thetadatadx';

async function main() {
  // Pass your API key directly. Add historicalType: "STAGE" to target staging.
  const client = await Client.connectWith({ apiKey: 'td1_...' });

  await client.stream.startStreaming((event) => {
    if (event.kind === 'trade' && event.trade) {
      const { contract, price, size, exchange, msOfDay, sequence, condition } = event.trade;
      console.log(
        `${contract.symbol} ${contract.expiration} ${contract.strike} ${contract.right} trade price=${price} size=${size} ` +
        `exchange=${exchange} ms_of_day=${msOfDay} sequence=${sequence} condition=${condition}`,
      );
    } else if (event.kind === 'quote' && event.quote) {
      const { contract, bid, ask, bidSize, askSize, bidExchange, askExchange, msOfDay } = event.quote;
      console.log(
        `${contract.symbol} ${contract.expiration} ${contract.strike} ${contract.right} quote bid=${bid} ask=${ask} ` +
        `bid_size=${bidSize} ask_size=${askSize} bid_exchange=${bidExchange} ` +
        `ask_exchange=${askExchange} ms_of_day=${msOfDay}`,
      );
    }
  });

  const leg = { expiration: '20260619', strike: '550', right: 'C' };
  client.stream.subscribeMany([
    Contract.option('SPY', leg).quote(),
    Contract.option('SPY', leg).trade(),
  ]);
}

await main();
```

Other ways to construct the client:

```typescript
import { Client } from 'thetadatadx';

// API key from the THETADATA_API_KEY environment variable, or from a .env file
const fromEnv = await Client.connectWith({ apiKeyFromEnv: true });
const fromDotenv = await Client.connectWith({ apiKeyFromDotenv: '.env' });

// Email and password, inline
const withLogin = await Client.connectWith({ email: 'you@example.com', password: 'your_password' });

// Full control: load a typed credentials file (custom hosts, timeouts via Config)
const fullControl = await Client.connectFromFile('creds.txt');
```

### C++

```cpp
#include <thetadatadx.hpp>
#include <cstdio>

int main() {
    // Pass your API key directly. Add .stage() before .connect() for staging.
    auto client = thetadatadx::Client::builder()
        .api_key("td1_...")
        .connect();

    auto greeks = client.historical().option_history_greeks_first_order("SPY", "20260619", "20240315");
    for (const auto& t : greeks) {
        std::printf("K=%.2f %c delta=%+.4f gamma=%+.4f\n",
                    t.strike, t.right, t.delta, t.gamma);
    }
}
```

### Rust

```toml
[dependencies]
thetadatadx = "13.0.0-rc.13"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use thetadatadx::Client;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    // Pass your API key directly. Add .stage() before .connect() for staging.
    let client = Client::builder().api_key("td1_...").connect().await?;

    let greeks = client
        .historical()
        .option_history_greeks_eod("SPY", "20260619", "20240101", "20240331")
        .await?;

    for t in greeks.iter().take(5) {
        println!("{} K={:.2} {} delta={:+.4}", t.date, t.strike, t.right, t.delta);
    }
    Ok(())
}
```

## DataFrames

Every historical result is a typed list that converts directly to a dataframe:
no row-by-row iteration:

```python
greeks.to_polars()   # polars.DataFrame
greeks.to_pandas()   # pandas.DataFrame   (pip install thetadatadx[pandas])
greeks.to_arrow()    # pyarrow.Table      (zero-copy)
```

The same `.to_polars()` / `.to_pandas()` / `.to_arrow()` terminals are available
on flat-file results. For multi-day backfills, stream the response in chunks
instead of buffering it. See [Request sizing](https://userfrm.github.io/ThetaDataDx/articles/request-sizing).

## Streaming

One connection, one authentication. Historical queries work immediately; the
streaming transport connects on the first subscription. Subscribe specific
contracts with the fluent `Contract` API, or take a whole-market feed: every
option trade across the universe, no per-contract setup. The full-trade feed
sends a quote and an OHLC bar before each trade, so add an `Ohlcvc` case to the
callback to handle the bars:

```python
from thetadatadx import Ohlcvc

def on_full_trade(event):
    match event:
        case Ohlcvc(open=o, high=h, low=lo, close=cl, volume=v, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} bar "
                f"o={o:.2f} h={h:.2f} l={lo:.2f} c={cl:.2f} volume={v}"
            )
        case _:
            on_event(event)   # reuse the quote/trade handling above

with client.streaming(on_full_trade) as session:
    session.subscribe(SecType.OPTION.full_trades())
    time.sleep(60)   # the callback runs on the streaming thread; keep it fast
```

> [!TIP]
> The callback above is one of two delivery modes. It pushes one typed event at
> a time for the lowest latency, ideal when you react to each trade or quote.
> For bulk and analytics, `client.stream.batches(...)` delivers the same
> subscriptions as Apache Arrow `RecordBatch` values under a fixed schema, ready
> to pull into pandas, Polars, or DuckDB. Open the reader first, since it starts
> the session, then subscribe. Every binding has both; the
> [streaming guide](https://userfrm.github.io/ThetaDataDx/streaming/) covers them.

> [!TIP]
> On an involuntary disconnect the client recovers on its own: exponential
> backoff with jitter, automatic host failover, then a paced re-subscribe of
> every active contract. Read liveness directly off the stream with
> `connection_status()` and the last-event timestamp; no separate health poll
> needed.

## Endpoint coverage

65 typed endpoints across stocks, options, indices, the market calendar, and
interest rates, plus real-time streaming.

| Category | Endpoints | Examples |
|---|---|---|
| Stock | 16 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 36 | Every stock surface plus five Greeks tiers, open interest, contract lists |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holidays, early closes |
| Interest rate | 1 | EOD rate history |

The full per-language method list lives in the
[API Reference](https://userfrm.github.io/ThetaDataDx/reference/).

## Errors

Every binding raises the same typed hierarchy, so the same cases are catchable
in any language: `AuthenticationError`, `RateLimitError`, `NotFoundError`,
`DeadlineExceededError`, `InvalidParameterError`, and the rest, all under a
common `ThetaDataError` base.

## Repository layout

| Path | Package | Purpose |
|---|---|---|
| [`thetadatadx-rs`](thetadatadx-rs/) | `thetadatadx` (crates.io) | The Rust SDK: tick types, decoders, and the network client in one crate |
| [`thetadatadx-py`](thetadatadx-py/) | `thetadatadx` (PyPI) | Python package with DataFrame adapters |
| [`thetadatadx-ts`](thetadatadx-ts/) | `thetadatadx` (npm) | TypeScript / Node.js package, prebuilt binaries |
| [`thetadatadx-cpp`](thetadatadx-cpp/) | header + prebuilt library | C++ wrapper over the C ABI |
| [`thetadatadx-ffi`](thetadatadx-ffi/) | release artifacts | C ABI for embedders |
| [`tools/server`](tools/server/) | `thetadatadx-server` | Local HTTP / WebSocket server |
| [`tools/mcp`](tools/mcp/) | `thetadatadx-mcp` (npm) | MCP server exposing every historical endpoint to AI clients |
| [`docs-site`](docs-site/) | — | Documentation site (GitHub Pages) |

## Documentation

- [Documentation site](https://userfrm.github.io/ThetaDataDx/): getting started, API reference, streaming, server, and MCP
- [Changelog](CHANGELOG.md)

## Roadmap

See [ROADMAP.md](ROADMAP.md) for where the project is headed. Up next: a [native Go SDK](https://github.com/userFRM/ThetaDataDx/issues/1019) and a [self-updating server](https://github.com/userFRM/ThetaDataDx/issues/957). The MCP server now runs straight from npm — `npx -y thetadatadx-mcp`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and the
pull-request process. Community discussion happens on the
[ThetaData Discord](https://discord.thetadata.us/).

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](./LICENSE).
