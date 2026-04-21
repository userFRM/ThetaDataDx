---
title: Rust Quickstart
description: Install, authenticate, run a historical call, subscribe to streaming, and handle errors with ThetaDataDx in Rust.
---

# Rust Quickstart

Native Rust client, `tokio`-based, no FFI. The same crate the other four SDKs bind against.

## Install

```toml
# Cargo.toml
[dependencies]
thetadatadx = "7.3"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Authenticate

Credentials file (`creds.txt` with email on line 1, password on line 2) or environment variables:

```rust
use thetadatadx::Credentials;

// From file
let creds = Credentials::from_file("creds.txt")?;

// Or from env vars
let creds = Credentials::new(
    std::env::var("THETA_EMAIL")?,
    std::env::var("THETA_PASS")?,
);
```

## Connect

```rust
use thetadatadx::{ThetaDataDx, DirectConfig};

let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```

## Historical call

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={:.2} H={:.2} L={:.2} C={:.2} V={}",
            tick.date, tick.open, tick.high, tick.low, tick.close, tick.volume);
    }
    Ok(())
}
```

## Streaming call

The Rust SDK uses a synchronous callback driven by a ring-reader thread. Pairs naturally with `tokio::spawn_blocking` if you need to consume events in an async task.

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    tdx.start_streaming(|event: &FpssEvent| match event {
        FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
            println!("Quote: {} {bid:.2}/{ask:.2}", contract.root);
        }
        FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
            println!("Trade: {} {price:.2} x {size}", contract.root);
        }
        _ => {}
    })?;

    tdx.subscribe_quotes(&Contract::stock("AAPL"))?;

    tokio::signal::ctrl_c().await.ok();
    tdx.stop_streaming();
    Ok(())
}
```

## Error handling

```rust
use thetadatadx::Error;

match tdx.option_history_greeks_all("SPY", "20240419", "500", "C",
                                    "20240101", "20240301").await {
    Ok(ticks) => process(ticks),
    Err(Error::RateLimited { wait_ms, .. }) => {
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        // retry
    }
    Err(Error::Subscription { endpoint, required_tier }) => {
        eprintln!("{endpoint} requires {required_tier}");
    }
    Err(err) => return Err(err),
}
```

## Next

- [Historical data](../historical/) — 61 endpoints
- [Streaming (FPSS)](../streaming/) — callback model, ring buffer, reconnect
- [Options & Greeks](../options) — wildcard chain queries, local Greeks calculator
- [Error handling](../getting-started/errors) — full `ThetaDataError` hierarchy
