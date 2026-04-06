---
title: Quick Start
description: Run your first ThetaDataDx query - fetch stock data, list option expirations, and compute Greeks.
---

# Quick Start

This page walks through a complete first query that fetches stock data, lists option expirations, and computes Greeks offline.

## First Query

::: code-group
```rust [Rust]
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
            tick.date, tick.open, tick.high,
            tick.low, tick.close, tick.volume);
    }

    // List option expirations
    let exps = client.option_list_expirations("SPY").await?;
    println!("SPY expirations: {:?}", &exps[..5.min(exps.len())]);

    // Compute Greeks (offline, no server call)
    let greeks = tdbe::greeks::all_greeks(
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
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

# Authenticate and connect
creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())

# Fetch end-of-day stock data
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} H={tick['high']:.2f} "
          f"L={tick['low']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# List option expirations
exps = client.option_list_expirations("SPY")
print(f"SPY expirations: {exps[:5]}")

# Compute Greeks (offline, no server call)
from thetadatadx import all_greeks

g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05,
    div_yield=0.015, tte=30/365, option_price=8.50, is_call=True
)
print(f"IV={g['iv']:.4f} Delta={g['delta']:.4f} Gamma={g['gamma']:.6f}")
```
```go [Go]
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    // Load credentials
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil {
        log.Fatal(err)
    }
    defer creds.Close()

    // Connect
    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Fetch end-of-day data
    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil {
        log.Fatal(err)
    }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
    }

    // Compute Greeks (offline, no server call)
    g, err := thetadatadx.AllGreeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("IV=%.4f Delta=%.4f Gamma=%.6f\n", g.IV, g.Delta, g.Gamma)
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iostream>
#include <iomanip>

int main() {
    // Load credentials
    auto creds = tdx::Credentials::from_file("creds.txt");

    // Connect
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    // Fetch end-of-day data
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (auto& tick : eod) {
        std::cout << tick.date << ": O=" << std::fixed << std::setprecision(2)
                  << tdx::open_f64(tick) << " H=" << tdx::high_f64(tick)
                  << " L=" << tdx::low_f64(tick) << " C=" << tdx::close_f64(tick) << std::endl;
    }

    // Compute Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
    std::cout << "IV=" << g.iv << " Delta=" << g.delta
              << " Gamma=" << g.gamma << std::endl;
}
```
:::

### Sample Response

```json
[
  {"date": 20260302, "open": 678.62, "high": 688.62, "low": 678.02, "close": 686.38, "volume": 87115983, "bid": 685.86, "ask": 685.90},
  {"date": 20260303, "open": 674.95, "high": 682.61, "low": 669.66, "close": 680.33, "volume": 104510616, "bid": 679.20, "ask": 679.26},
  {"date": 20260304, "open": 681.58, "high": 687.09, "low": 679.62, "close": 685.13, "volume": 78815016, "bid": 685.45, "ask": 685.60}
]
```

> SPY end-of-day data. Prices shown as f64 (decoded from fixed-point). All endpoints return structured data with the same JSON-like shape across SDKs.

## With pandas DataFrames (Python)

The Python SDK provides convenience methods that return pandas DataFrames directly:

```python
from thetadatadx import Credentials, Config, ThetaDataDx, to_dataframe

creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())

# Option 1: explicit conversion
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
print(df.head())

# Option 2: _df convenience methods
df = client.stock_history_eod_df("AAPL", "20240101", "20240301")
df = client.stock_history_ohlc_df("AAPL", "20240315", "60000")
```

Requires `pip install thetadatadx[pandas]`.

## What's Next

- [Historical Data](../historical/) - all 61 endpoints with examples
- [Real-Time Streaming](../streaming/) - FPSS subscribe/callback and polling
- [Options & Greeks](../options) - option chain workflow, local Greeks, and wildcard queries with contract identification
- [Configuration](../configuration) - timeouts, concurrency, server settings
- [API Reference](../api-reference) - complete type and method listing
