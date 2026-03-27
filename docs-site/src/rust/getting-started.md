# Getting Started with Rust

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
thetadatadx = "3.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Credentials

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

## First Query

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    // Load credentials
    let creds = Credentials::from_file("creds.txt")?;

    // Connect to ThetaData (authenticates automatically)
    let client = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    // Fetch end-of-day stock data
    let eod = client.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={} H={} L={} C={} V={}",
            tick.date, tick.open_price(), tick.high_price(),
            tick.low_price(), tick.close_price(), tick.volume);
    }

    // List option expirations
    let exps = client.option_list_expirations("SPY").await?;
    println!("SPY expirations: {:?}", &exps[..5.min(exps.len())]);

    // Compute Greeks (offline, no server call)
    let greeks = thetadatadx::greeks::all_greeks(
        450.0,        // spot
        455.0,        // strike
        0.05,         // risk-free rate
        0.015,        // dividend yield
        30.0 / 365.0, // time to expiry (years)
        8.50,         // option market price
        true,         // is_call
    );
    println!("IV: {:.4}, Delta: {:.4}, Gamma: {:.6}",
        greeks.iv, greeks.delta, greeks.gamma);

    Ok(())
}
```

## Credentials from Environment Variables

For containerized deployments or CI pipelines:

```rust
let creds = Credentials::new(
    std::env::var("THETA_EMAIL")?,
    std::env::var("THETA_PASS")?,
);
```

## What's Next

- [Historical Data](historical.md) -- all 61 endpoints with examples
- [Real-Time Streaming](streaming.md) -- FPSS subscribe/callback
- [Options & Greeks](options.md) -- option chain workflow and local Greeks
- [Configuration](configuration.md) -- timeouts, concurrency, server settings
- [API Reference](api-reference.md) -- complete type and method listing
