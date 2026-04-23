---
title: Quick Start
description: Install, authenticate, run a first historical call, subscribe to streaming, and move on. One page, every language.
---

# Quick Start

One page covering all five SDKs (Rust, Python, TypeScript / Node.js, Go, C++). Each step shows the same workflow tabbed across languages.

## Install

::: code-group
```bash [Rust]
# Cargo.toml
# [dependencies]
# thetadatadx = "8"
# tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

cargo add thetadatadx tokio --features tokio/rt-multi-thread,tokio/macros
```
```bash [Python]
pip install thetadatadx

# With DataFrame support:
pip install thetadatadx[pandas]   # pandas
pip install thetadatadx[polars]   # polars
pip install thetadatadx[arrow]    # pyarrow only
pip install thetadatadx[all]      # all three
```
```bash [TypeScript]
npm install thetadatadx
```
```bash [Go]
# Prerequisites: Go 1.21+, Rust toolchain, C compiler

git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi

go get github.com/userFRM/thetadatadx/sdks/go
```
```bash [C++]
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain

git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi

cd sdks/cpp
mkdir build && cd build
cmake ..
make
```
:::

Python wheels are pre-built (`abi3`, Python 3.9+); no Rust toolchain needed on supported platforms. Go and C++ link against the Rust FFI library built once with `cargo build`.

## Authenticate

::: code-group
```rust [Rust]
use thetadatadx::Credentials;

// From file (line 1 = email, line 2 = password)
let creds = Credentials::from_file("creds.txt")?;

// Or from env vars
let creds = Credentials::new(
    std::env::var("THETA_EMAIL")?,
    std::env::var("THETA_PASS")?,
);
```
```python [Python]
from thetadatadx import Credentials

# From file
creds = Credentials.from_file("creds.txt")

# Or from env vars
import os
creds = Credentials(os.environ["THETA_EMAIL"], os.environ["THETA_PASS"])
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

// Credentials are passed directly to the connect helpers below.
```
```go [Go]
import thetadatadx "github.com/userFRM/thetadatadx/sdks/go"

creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

// Or from env vars
envCreds, _ := thetadatadx.CredentialsFromEnv("THETA_EMAIL", "THETA_PASS")
defer envCreds.Close()
```
```cpp [C++]
#include "thetadx.hpp"

auto creds = tdx::Credentials::from_file("creds.txt");

// Or from env vars
auto envCreds = tdx::Credentials(
    std::getenv("THETA_EMAIL"),
    std::getenv("THETA_PASS")
);
```
:::

## First historical call

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = thetadatadx::Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={:.2} H={:.2} L={:.2} C={:.2} V={}",
            tick.date, tick.open, tick.high, tick.low, tick.close, tick.volume);
    }
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")

# Chain directly to a DataFrame:
df  = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_polars()
pdf = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_pandas()
tbl = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

const eod = tdx.stockHistoryEOD('AAPL', '20240101', '20240301');
for (const tick of eod) {
    console.log(`${tick.date}: O=${tick.open} H=${tick.high} L=${tick.low} C=${tick.close} V=${tick.volume}`);
}
```
```go [Go]
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil { log.Fatal(err) }
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil { log.Fatal(err) }
    defer client.Close()

    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil { log.Fatal(err) }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
    }
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iomanip>
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (const auto& tick : eod) {
        std::cout << tick.date
                  << std::fixed << std::setprecision(2)
                  << ": O=" << tick.open << " H=" << tick.high
                  << " L=" << tick.low  << " C=" << tick.close
                  << " V=" << tick.volume << std::endl;
    }
}
```
:::

Every historical endpoint returns typed tick records (`EodTick`, `OhlcTick`, `TradeTick`, ...). On Python the returned list wrapper chains directly to `to_polars()`, `to_pandas()`, `to_arrow()`, or `to_list()`. See [DataFrames](./dataframes) for the zero-copy Arrow scope.

## First streaming call

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = thetadatadx::Credentials::from_file("creds.txt")?;
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
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")

try:
    while True:
        event = tdx.next_event(timeout_ms=1000)
        if event is None:
            continue
        if event.kind == "quote":
            print(f"Quote: {event.contract_id} "
                  f"{event.bid:.2f}/{event.ask:.2f}")
        elif event.kind == "trade":
            print(f"Trade: {event.contract_id} "
                  f"{event.price:.2f} x {event.size}")
        elif event.kind == "simple" and event.event_type == "disconnected":
            break
finally:
    tdx.stop_streaming()
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

tdx.startStreaming();
tdx.subscribeQuotes('AAPL');
tdx.subscribeTrades('MSFT');

try {
    while (true) {
        const event = tdx.nextEvent(1000);
        if (!event) continue;

        if (event.kind === 'quote') {
            console.log(`Quote: ${event.contractId} ${event.bid.toFixed(2)}/${event.ask.toFixed(2)}`);
        } else if (event.kind === 'trade') {
            console.log(`Trade: ${event.contractId} ${event.price.toFixed(2)} x ${event.size}`);
        } else if (event.kind === 'simple' && event.eventType === 'disconnected') {
            break;
        }
    }
} finally {
    tdx.stopStreaming();
}
```
```go [Go]
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil { log.Fatal(err) }
    defer fpss.Close()

    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeTrades("MSFT")

    for {
        event, err := fpss.NextEvent(1000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil { continue }

        switch event.Kind {
        case thetadatadx.FpssQuoteEvent:
            q := event.Quote
            fmt.Printf("Quote: %d %.2f/%.2f\n", q.ContractID, q.Bid, q.Ask)
        case thetadatadx.FpssTradeEvent:
            t := event.Trade
            fmt.Printf("Trade: %d %.2f x %d\n", t.ContractID, t.Price, t.Size)
        }
    }

    fpss.Shutdown()
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();
    tdx::FpssClient fpss(creds, config);

    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_trades("MSFT");

    while (true) {
        auto event = fpss.next_event(1000);
        if (!event) continue;

        switch (event->kind) {
        case TDX_FPSS_QUOTE: {
            const auto& q = event->quote;
            std::cout << "Quote: " << q.contract_id
                      << " " << q.bid << "/" << q.ask << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            const auto& t = event->trade;
            std::cout << "Trade: " << t.contract_id
                      << " " << t.price << " x " << t.size << std::endl;
            break;
        }
        default: break;
        }
    }

    fpss.shutdown();
}
```
:::

Streaming is real-time FPSS — no polling the historical REST endpoints. See [Streaming (FPSS)](./streaming) for the callback / polling model, reconnect policy, and latency tracking.

## Where to next

- [Authentication](./authentication) — credentials file, environment variables, token lifecycle
- [First query](./first-query) — deeper dive on a single historical call
- [DataFrames](./dataframes) — Arrow / Polars / Pandas output with the zero-copy scope
- [Streaming (FPSS)](./streaming) — SPKI pinning, callback / polling models, lock-free ring, reconnect policy
- [Error handling](./errors) — `ThetaDataError` hierarchy, retry policy, session refresh
- [Historical endpoints](../historical/) — complete generated historical surface, with per-language examples on each
