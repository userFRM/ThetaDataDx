# thetadatadx

The Rust SDK for [ThetaData](https://thetadata.us) market data. Pull US stock, option, index, and rate data three ways — point-in-time **history**, real-time **streaming**, and whole-universe **flat files** — from one async client. Connects straight to ThetaData; nothing to install and run locally, no local proxy.

[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg?logo=rust)](https://crates.io/crates/thetadatadx)
[![docs.rs](https://img.shields.io/docsrs/thetadatadx?logo=docsdotrs)](https://docs.rs/thetadatadx)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE)

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. The SDK
> authenticates against ThetaData's Nexus API using your account credentials.

## Features

- **Complete coverage** — stocks, options, indices, and rates across 65 typed endpoints.
- **Three access modes, one client** — point-in-time history, real-time streaming, and bulk flat-file downloads.
- **Greeks without a round-trip** — first- through third-order Black-Scholes Greeks and an implied-volatility solver, computed locally.
- **Buffer or stream** — every history builder yields a `Vec<Tick>` on `.await`, or chunk-by-chunk via `.stream(handler)`.
- **Typed errors** — one `Error` enum across every transport, plus a dedicated `FpssError` for the streaming path.
- **DataFrames on demand** — opt into the `polars` / `arrow` features for a zero-copy conversion off any result.

## Install

```toml
[dependencies]
thetadatadx = "12"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Opt into DataFrame ergonomics with the `polars` or `arrow` feature:

```toml
thetadatadx = { version = "12", features = ["polars"] }
```

## Quick start

> [!TIP]
> Credentials can come from a `creds.txt` file (email on line 1, password on
> line 2), an inline `Credentials::new(...)`, or the `THETADATA_EMAIL` /
> `THETADATA_PASSWORD` environment variables.

```rust
use thetadatadx::{Client, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = Client::connect(&creds, DirectConfig::production()).await?;

    // EOD Greeks for a SPY option chain across Q1 2024.
    let chain = client
        .historical()
        .option_history_greeks_eod("SPY", "20260619", "20240101", "20240331")
        .await?;

    for t in chain.iter().take(5) {
        println!(
            "{} K={:>7.2} {} delta={:+.4} gamma={:+.4} theta={:+.4} IV={:.4}",
            t.date, t.strike, t.right, t.delta, t.gamma, t.theta, t.implied_volatility,
        );
    }
    Ok(())
}
```

65 typed endpoints span stocks, options, indices, the market calendar, and interest rates. Each builder accepts `.await` for a buffered `Vec<Tick>`, or `.stream(handler)` for chunk-by-chunk delivery — the right choice for multi-day backfills, where it holds peak memory flat instead of materialising the whole response.

## Streaming

One authentication, one connection. Historical queries work immediately; the streaming transport opens on the first `start_streaming(callback)` call. Subscribe specific contracts with the fluent `Contract` API, or take a whole-market feed:

```rust
use thetadatadx::fpss::{StreamData, StreamEvent};
use thetadatadx::prelude::*;

fn format_contract(contract: &Contract) -> String {
    let mut label = contract.symbol.to_string();
    if let Some(expiration) = contract.expiration {
        label.push_str(&format!(" {expiration}"));
    }
    if let Some(strike) = contract.strike_dollars() {
        label.push_str(&format!(" {strike}"));
    }
    if let Some(right) = contract.right() {
        label.push_str(&format!(" {}", right.as_char()));
    }
    label
}

client.stream().start_streaming(|event: &StreamEvent| {
    match event {
        StreamEvent::Data(StreamData::Trade {
            contract,
            price,
            size,
            exchange,
            ms_of_day,
            sequence,
            condition,
            ..
        }) => {
            println!(
                "{} trade price={price} size={size} exchange={exchange} ms_of_day={ms_of_day} sequence={sequence} condition={condition}",
                format_contract(contract),
            );
        }
        StreamEvent::Data(StreamData::Quote {
            contract,
            bid,
            ask,
            bid_size,
            ask_size,
            bid_exchange,
            ask_exchange,
            ms_of_day,
            ..
        }) => {
            println!(
                "{} quote bid={bid} ask={ask} bid_size={bid_size} ask_size={ask_size} bid_exchange={bid_exchange} ask_exchange={ask_exchange} ms_of_day={ms_of_day}",
                format_contract(contract),
            );
        }
        _ => {}
    }
})?;

client.stream().subscribe(Contract::stock("AAPL").quote())?;
client.stream().subscribe(
    Contract::option("SPY", OptionLeg { expiration: "20260620", strike: "550", right: "C" })?
        .trade(),
)?;

// Or a whole-market feed — every option trade across the universe.
client.stream().subscribe(SecType::Option.full_trades())?;
```

On an involuntary disconnect the client recovers on its own — exponential backoff with jitter, host failover, then a paced re-subscribe of every active contract.

### Streaming-only — `StreamingClient`

For workloads that never touch history, `StreamingClient` connects to the streaming servers alone — no Nexus session, no historical surface. Drive it as an iterator, or with the explicit drain primitives:

```rust
use thetadatadx::auth::Credentials;
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamingClient, StreamEvent};

let creds = Credentials::from_file("creds.txt")?;
let hosts = DirectConfig::production().streaming.hosts;
let client = StreamingClient::builder(&creds, &hosts).ring_size(8192).build()?;

client.subscribe(Contract::stock("AAPL").quote())?;

for event in &client {
    match event? {
        StreamEvent::Data(data)       => { /* market-data tick */ }
        StreamEvent::Control(control) => { /* lifecycle event   */ }
        _ => {}
    }
}
```

Beyond the iterator, `StreamingClient` exposes `next_event()` (blocking pop), `try_next_event()` (non-blocking pop), `poll_batch(|e| …)` (non-blocking batch drain), and `for_each(|e| …)` (blocking loop until shutdown). Auto-reconnect with subscription replay is on by default; tune it through `StreamingClientBuilder` or `DirectConfig::reconnect`.

## Flat files

Whole-universe daily snapshots for one `(security type, request type, date)` at a time, written straight to disk in the format you ask for:

```rust
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};

let path = thetadatadx::flatfile_request(
    &creds, SecType::Option, ReqType::TradeQuote, "20260428",
    std::path::Path::new("/tmp/option-trade-quote"), FlatFileFormat::Csv,
).await?;
```

## Greeks calculator

A full Black-Scholes calculator — first- through third-order Greeks plus an implied-volatility solver — runs locally, no request required.

## DataFrames

With the `polars` or `arrow` feature enabled, any history result converts to a dataframe over the Arrow C Data Interface — zero-copy, no row-by-row iteration:

```rust
let df = client.historical().stock_history_eod("AAPL", "20240101", "20240301").await?.to_polars()?;
```

## Errors

Every public method returns `Result<_, thetadatadx::Error>`. The streaming surface adds a typed `FpssError` for the polling and subscribe paths; `Error` implements `From<FpssError>`, so callers that prefer a single error type can stay with `Error` throughout.

## Documentation

- [API reference](https://docs.rs/thetadatadx)
- [Documentation site](https://userfrm.github.io/ThetaDataDx/) — guides, per-endpoint pages, and a streaming deep-dive
- [Repository, issues, contributing](https://github.com/userFRM/ThetaDataDx)

Python (`pip install thetadatadx`), Node.js (`npm install thetadatadx`), and C++ bindings sit on this same engine and share its wire format and reconnect behaviour.

## License

Licensed under the Apache License, Version 2.0.
