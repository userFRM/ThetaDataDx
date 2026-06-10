---
title: Real-Time Streaming
description: Overview of ThetaDataDx real-time streaming - architecture, SDK models, and getting started.
---

# Real-Time Streaming

Real-time market data is delivered over a persistent TLS/TCP streaming channel into the SDK. The streaming channel carries live quotes, trades, open interest, and OHLCVC bars as typed, zero-copy events.

## Architecture

```mermaid
graph LR
    A["Exchange<br/>(NYSE/NASDAQ)"] --> B["ThetaData streaming<br/>servers (4 NJ hosts)"]
    B -->|"TLS/TCP"| C["SDK I/O Thread<br/>(FIT decode)"]
    C -->|"SPSC<br/>ring buffer"| D["Your Application<br/>(callback / poll)"]
```

Events are decoded from the FIT wire format and delta-decompressed on an I/O thread, then dispatched through a single-producer single-consumer ring buffer to your callback. Every data event carries a `received_at_ns` nanosecond timestamp captured at frame decode time.

## Client Model

Streaming is delivered through a unified client in every binding. The surface is identical across languages: one connect call, one `subscribe()` polymorphic over typed subscription specs, one push-callback delivery path backed by the same streaming ring on the Rust side.

| SDK | Streaming client | Entry point |
|-----|------------------|-------------|
| **Rust** | `ThetaDataDxClient` (the main client) | `start_streaming(callback)` |
| **Python** | `ThetaDataDxClient` (the main client) | `start_streaming(callback)` or the `streaming(callback)` context manager |
| **TypeScript/Node.js** | `ThetaDataDxClient` (the main client) | `startStreaming(callback)` |
| **C++** | `tdx::UnifiedClient` (the main client) | `set_callback(lambda)` |

::: tip
If you are porting code between SDKs: anywhere a Rust example calls `client.subscribe(Contract::stock("AAPL").quote())`, the Python / TypeScript / C++ equivalents call the same polymorphic `subscribe(...)` on the same client type with the same typed subscription spec.
:::

## SDK Streaming Models

| SDK | Push (callback) | Event shape |
|-----|-----------------|-------------|
| **Rust** | `client.start_streaming(\|event\| ...)` | `&FpssEvent` enum |
| **Python** | `client.start_streaming(callback)` | typed pyclass per `FpssData` / `FpssControl` variant |
| **TypeScript/Node.js** | `client.startStreaming(callback)` | JS object discriminated on `event.kind` |
| **C++** | `client.set_callback(lambda)` | `TdxFpssEvent` — `#[repr(C)]` tagged union |

::: warning No JSON in FFI
C++ receives typed `#[repr(C)]` structs directly from Rust -- not JSON. All field access is zero-copy struct member access.
:::

## Embedded Rust iterator

The Rust `FpssClient` also implements `Iterator` directly. Build the client with `FpssClient::builder(creds, hosts).build()?` and drive it from your own loop:

```rust [Rust]
use thetadatadx::fpss::{FpssClient, FpssEvent, FpssData};
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::auth::Credentials;

fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let hosts = thetadatadx::config::DirectConfig::production().fpss.hosts;
    let client = FpssClient::builder(&creds, &hosts).build()?;
    client.subscribe(Contract::stock("AAPL").quote())?;

    for event in &client {
        match event {
            Ok(FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. })) => {
                println!("Quote: {} bid={bid:.2} ask={ask:.2}", contract.symbol);
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("stream error: {e}");
                break;
            }
        }
    }
    Ok(())
}
```

For non-blocking integrations, drive `client.poll_batch(|event| ...)` from your own scheduler (returns `PollOutcome::Drained(n)` or `PollOutcome::Shutdown`) or block the calling thread on `client.for_each(|event| ...)` until the ring shuts down. `client.next_event() -> Result<Option<FpssEvent>, FpssError>` yields one event at a time when you need explicit per-event control. `client.shutdown()` (or dropping the client) ends every drain path once the ring is fully consumed, so no in-flight events are lost.

## Available Data Streams

| Stream | Event Type | Description |
|--------|------------|-------------|
| Quotes | `Quote` | Real-time NBBO bid/ask updates (11 fields + `received_at_ns`) |
| Trades | `Trade` | Individual trade executions (16 fields + `received_at_ns`) |
| Open Interest | `OpenInterest` | Current open interest for options (3 fields + `received_at_ns`) |
| OHLCVC | `Ohlcvc` | Aggregated OHLC bars with volume (`i64`) and count (`i64`) |
| Full Trades | `Trade` | All trades for an entire security type (full-stream subscription) |
| Full OI | `OpenInterest` | All open interest for an entire security type (full-stream subscription) |

::: tip Full-stream subscriptions are Stock and Option only
The full-stream subscription (`full_trades` / `full_open_interest`) is broadcast for the Stock and Option security types only. Indices and rates have no full-stream broadcast upstream — subscribe to them per-contract instead, e.g. `Contract::index("VIX").trade()`. A full-stream subscription on any other security type is rejected with a configuration error when you call `subscribe`.
:::

## Event Categories

Events are either **data** (market ticks) or **control** (lifecycle/protocol):

- **Data events**: `Quote`, `Trade`, `OpenInterest`, `Ohlcvc` -- every one carries `received_at_ns`
- **Control events**: `LoginSuccess`, `ContractAssigned`, `ReqResponse`, `MarketOpen`, `MarketClose`, `ServerError`, `Disconnected`, `Error`
- **UnknownFrame**: typed control variant the SDK emits for any frame whose wire code is not yet recognised; carries the raw `code: u8` and `payload: Vec<u8>` for diagnostic logging.

## Flush Mode

`FpssFlushMode` controls when the TCP write buffer is flushed:

| Mode | Behavior | Latency | Syscall overhead |
|------|----------|---------|-----------------|
| `Batched` (default) | Flush only on PING frames (one ping interval, default ~250ms) | Up to one ping interval additional | Lower |
| `Immediate` | Flush after every frame write | Lowest possible | Higher |

## Quick Start

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;


#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    client.start_streaming(|event: &FpssEvent| {
        match event {
            FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
                println!("Quote: {} bid={bid:.2} ask={ask:.2}", contract.symbol);
            }
            FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
                println!("Trade: {} price={price:.2} size={size}", contract.symbol);
            }
            _ => {}
        }
    })?;

    client.subscribe(Contract::stock("AAPL").quote())?;
    client.subscribe(Contract::stock("MSFT").trade())?;

    std::thread::park(); // block until interrupted
    client.stop_streaming();
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract

creds = Credentials.from_file("creds.txt")
client = ThetaDataDxClient(creds, Config.production())

# `streaming(callback)` is a context manager that registers the
# callback on enter and pairs `stop_streaming()` + `await_drain()`
# on exit. The dispatcher thread invokes `on_event(event)`
# for every typed FPSS event.
def on_event(event):
    if event.kind == "quote":
        print(f"Quote: {event.contract.symbol} "
              f"bid={event.bid:.2f} ask={event.ask:.2f}")
    elif event.kind == "trade":
        print(f"Trade: {event.contract.symbol} "
              f"price={event.price:.2f} size={event.size}")

with client.streaming(on_event):
    client.subscribe(Contract.stock("AAPL").quote())
    client.subscribe(Contract.stock("MSFT").trade())
    import time
    time.sleep(60)
```
```cpp [C++]
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();
    auto client = tdx::UnifiedClient::connect(creds, config);

    client.set_callback([](const tdx::FpssEvent& event) {
        switch (event.kind) {
        case TDX_FPSS_QUOTE: {
            auto& q = event.quote;
            std::cout << "Quote: " << q.contract.symbol
                      << " bid=" << q.bid << " ask=" << q.ask << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            auto& t = event.trade;
            std::cout << "Trade: " << t.contract.symbol
                      << " price=" << t.price << " size=" << t.size << std::endl;
            break;
        }
        default: break;
        }
    });

    client.subscribe(tdx::Contract::stock("AAPL").quote());
    client.subscribe(tdx::Contract::stock("MSFT").trade());

    // ... let the callback run ...
    client.stop_streaming();
}
```
:::

## Server Environments

| Config | Streaming Ports | Purpose |
|--------|-----------|---------|
| `DirectConfig::production()` | 20000, 20001 | Live production data |
| `DirectConfig::dev()` | 20200, 20201 | Historical day replay at max speed (markets closed testing) |
| `DirectConfig::stage()` | 20100, 20101 | Staging/testing (frequent reboots, unstable) |

Streaming hosts are configurable -- not hardcoded. Override `fpss_hosts` on `DirectConfig` or use a TOML config file.

## Next Steps

1. [Connecting & Subscribing](./connection) -- establish a streaming connection, choose server environment, configure flush mode
2. [Handling Events](./events) -- process data and control events with full field reference tables
3. [Latency Measurement](./latency) -- use `received_at_ns` and `tdbe::latency::latency_ns()` for wire-to-application latency
4. [Reconnection & Error Handling](./reconnection) -- handle disconnects with `reconnect_streaming()` or manual recovery
