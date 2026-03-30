---
title: Getting Started
description: Install ThetaDataDx, configure credentials, and run your first query across Rust, Python, Go, and C++.
---

# Getting Started

## Installation

::: code-group
```toml [Rust]
# Add to your Cargo.toml
[dependencies]
thetadatadx = "3.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```
```bash [Python]
pip install thetadatadx

# With DataFrame support:
pip install thetadatadx[pandas]    # pandas DataFrames
pip install thetadatadx[polars]    # polars DataFrames
pip install thetadatadx[all]       # both

# Requires Python 3.9+. Pre-built wheels are provided - no Rust toolchain required.
```
```bash [Go]
# Prerequisites: Go 1.21+, Rust toolchain, C compiler (for CGo)

# First, build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.so (Linux)
# or libthetadatadx_ffi.dylib (macOS)

# Then add the Go module:
go get github.com/userFRM/ThetaDataDx/sdks/go
```
```bash [C++]
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain

# First, build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.so (Linux)
# or libthetadatadx_ffi.dylib (macOS)

# Then build the C++ SDK:
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```
:::

### Building Python from Source

For unsupported platforms where pre-built wheels are not available:

```bash
pip install maturin
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/python
maturin develop --release
```

## Credentials

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

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

    thetadatadx "github.com/userFRM/ThetaDataDx/sdks/go"
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
#include "thetadatadx.hpp"
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
                  << tick.open << " H=" << tick.high
                  << " L=" << tick.low << " C=" << tick.close << std::endl;
    }

    // Compute Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
    std::cout << "IV=" << g.iv << " Delta=" << g.delta
              << " Gamma=" << g.gamma << std::endl;
}
```
:::

## Credentials from Environment Variables

For containerized deployments or CI pipelines:

::: code-group
```rust [Rust]
let creds = Credentials::new(
    std::env::var("THETA_EMAIL")?,
    std::env::var("THETA_PASS")?,
);
```
```python [Python]
import os
from thetadatadx import Credentials

creds = Credentials(os.environ["THETA_EMAIL"], os.environ["THETA_PASS"])
```
```go [Go]
creds, err := thetadatadx.CredentialsFromEnv("THETA_EMAIL", "THETA_PASS")
if err != nil {
    log.Fatal(err)
}
defer creds.Close()
```
```cpp [C++]
auto creds = tdx::Credentials(
    std::getenv("THETA_EMAIL"),
    std::getenv("THETA_PASS")
);
```
:::

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

## Memory Management

### Go

All Go SDK objects that wrap FFI handles must be closed when no longer needed:

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()  // frees the Rust-side allocation

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()
```

### C++

The C++ SDK uses RAII wrappers around the C FFI handles. All objects automatically free their resources when they go out of scope. No manual memory management required.

```cpp
{
    auto client = tdx::Client::connect(creds, tdx::Config::production());
    // ... use client ...
}  // client automatically freed here
```

All methods throw `std::runtime_error` on failure.

## What's Next

- [Historical Data](historical.md) - all 61 endpoints with examples
- [Real-Time Streaming](streaming.md) - FPSS subscribe/callback and polling
- Options & Greeks - option chain workflow and local Greeks
- Configuration - timeouts, concurrency, server settings
- API Reference - complete type and method listing
