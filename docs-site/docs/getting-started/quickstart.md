---
title: Quick Start
description: Install, authenticate, run a first historical call, subscribe to streaming, and move on. One page, every language.
---

# Quick Start

One page covering all four SDKs (Rust, Python, TypeScript / Node.js, C++). Each step shows the same workflow tabbed across languages. Go consumers can build a thin cgo wrapper against the unchanged C ABI in [`ffi/`](https://github.com/userFRM/ThetaDataDx/tree/main/ffi).

## Install

::: code-group
```bash [Rust]
# Cargo.toml
# [dependencies]
# thetadatadx = "11"
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

Python wheels are pre-built (`abi3`, Python 3.12+); no Rust toolchain needed on supported platforms. C++ links against the Rust FFI library built once with `cargo build`.

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
import { ThetaDataDxClient } from 'thetadatadx';

// Credentials are passed directly to the connect helpers below.
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
use thetadatadx::{ThetaDataDxClient, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = thetadatadx::Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={:.2} H={:.2} L={:.2} C={:.2} V={}",
            tick.date, tick.open, tick.high, tick.low, tick.close, tick.volume);
    }
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

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
import { ThetaDataDxClient } from 'thetadatadx';

const tdx = await ThetaDataDxClient.connectFromFile('creds.txt');

const eod = tdx.stockHistoryEOD('AAPL', '20240101', '20240301');
for (const tick of eod) {
    console.log(`${tick.date}: O=${tick.open} H=${tick.high} L=${tick.low} C=${tick.close} V=${tick.volume}`);
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iomanip>
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::UnifiedClient::connect(creds, tdx::Config::production());

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
use thetadatadx::{ThetaDataDxClient, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = thetadatadx::Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    tdx.start_streaming(|event: &FpssEvent| match event {
        FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
            println!("Quote: {} {bid:.2}/{ask:.2}", contract.symbol);
        }
        FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
            println!("Trade: {} {price:.2} x {size}", contract.symbol);
        }
        _ => {}
    })?;

    // Fluent contract-first subscribe — `Contract::stock("AAPL").quote()`
    // produces a `Subscription` the unified `subscribe()` accepts.
    tdx.subscribe(Contract::stock("AAPL").quote())?;
    tdx.subscribe(Contract::stock("MSFT").trade())?;

    tokio::signal::ctrl_c().await.ok();
    tdx.stop_streaming();
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract

creds = Credentials.from_file("creds.txt")
client = ThetaDataDxClient(creds, Config.production())

client.subscribe(Contract.stock("AAPL").quote())
client.subscribe(Contract.stock("MSFT").trade())

# Pull-iter mode: context-managed typed iterator over the SPSC
# queue. The iterator raises StopIteration once `stop_streaming()`
# fires AND the queue is fully drained; the `with` block pairs
# `stop_streaming()` + `await_drain()` automatically on exit.
with client.streaming_iter() as it:
    for event in it:
        if event.kind == "quote":
            print(f"Quote: {event.contract.symbol} "
                  f"{event.bid:.2f}/{event.ask:.2f}")
        elif event.kind == "trade":
            print(f"Trade: {event.contract.symbol} "
                  f"{event.price:.2f} x {event.size}")
        elif event.kind == "disconnected":
            break
```
```typescript [TypeScript]
import { ThetaDataDxClient, Contract } from 'thetadatadx';

const client = await ThetaDataDxClient.connectFromFile('creds.txt');

client.subscribe(Contract.stock('AAPL').quote());
client.subscribe(Contract.stock('MSFT').trade());

// Pull-iter mode: async iterable over the SPSC queue. The
// iterator resolves `done: true` once `client.stopStreaming()`
// fires AND the queue is fully drained.
const iter = client.startStreamingIter();
try {
    for await (const event of iter) {
        if (event.kind === 'quote') {
            console.log(`Quote: ${event.contract.symbol} ${event.bid.toFixed(2)}/${event.ask.toFixed(2)}`);
        } else if (event.kind === 'trade') {
            console.log(`Trade: ${event.contract.symbol} ${event.price.toFixed(2)} x ${event.size}`);
        } else if (event.kind === 'simple' && event.eventType === 'disconnected') {
            break;
        }
    }
} finally {
    client.stopStreaming();
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();
    auto client = tdx::UnifiedClient::connect(creds, config);

    // Fluent contract-first subscribe — same shape as the Rust /
    // Python / TypeScript bindings.
    client.subscribe(tdx::Contract::stock("AAPL").quote());
    client.subscribe(tdx::Contract::stock("MSFT").trade());

    auto iter = client.start_streaming_iter();
    while (!iter.ended()) {
        auto event = iter.next(std::chrono::milliseconds(1000));
        if (!event) continue;

        switch (event->kind) {
        case TDX_FPSS_QUOTE: {
            const auto& q = event->quote;
            std::cout << "Quote: " << q.contract.symbol
                      << " " << q.bid << "/" << q.ask << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            const auto& t = event->trade;
            std::cout << "Trade: " << t.contract.symbol
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
