---
title: Real-Time Streaming
description: Subscribe to live market data via ThetaData's FPSS servers with quote, trade, open interest, and OHLC streaming across Rust, Python, Go, and C++.
---

# Real-Time Streaming

Real-time market data is delivered via ThetaData's FPSS (Feed Protocol Streaming Service) servers. FPSS delivers live quotes, trades, open interest, and OHLC snapshots over a persistent TLS/TCP connection.

Each SDK exposes FPSS differently:

- **Rust** -- Fully synchronous callback model. Events are dispatched through an LMAX Disruptor ring buffer. No Tokio on the streaming hot path.
- **Python** -- Polling model with `next_event()`. Events are returned as Python dicts.
- **Go** -- Polling model with `NextEvent()`. Events are returned as JSON.
- **C++** -- Polling model with `next_event()`. Events are returned as JSON strings. RAII handles cleanup automatically.

## Connect

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
            println!("Quote: contract={contract_id} bid={bid} ask={ask}");
        }
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
            println!("Trade: contract={contract_id} price={price} size={size}");
        }
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
            println!("Contract {id} = {contract}");
        }
        _ => {}
    }
})?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

fpss, _ := thetadatadx.NewFpssClient(creds, config)
defer fpss.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto config = tdx::Config::production();
tdx::FpssClient fpss(creds, config);
```
:::

The ring buffer size for event dispatch is configured via `DirectConfig` (Rust only).

## Subscribe

::: code-group
```rust [Rust]
// Stock quotes
let req_id = tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
println!("Subscribed (req_id={req_id})");

// Stock trades
tdx.subscribe_trades(&Contract::stock("MSFT"))?;

// Option quotes
let opt = Contract::option("SPY", 20261218, true, 60000); // call, strike $600
tdx.subscribe_quotes(&opt)?;

// Open interest
tdx.subscribe_open_interest(&Contract::stock("AAPL"))?;

// All trades for a security type
tdx.subscribe_full_trades(SecType::Stock)?;
```
```python [Python]
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")
tdx.subscribe_open_interest("SPY")
```
```go [Go]
// Stock quotes
reqID, _ := fpss.SubscribeQuotes("AAPL")
fmt.Printf("Subscribed (req_id=%d)\n", reqID)

// Stock trades
fpss.SubscribeTrades("MSFT")

// Open interest
fpss.SubscribeOpenInterest("AAPL")

// All trades for a security type
fpss.SubscribeFullTrades("STOCK")
```
```cpp [C++]
// Stock quotes
int32_t req_id = fpss.subscribe_quotes("AAPL");
std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

// Stock trades
fpss.subscribe_trades("MSFT");

// Open interest
fpss.subscribe_open_interest("AAPL");

// All trades for a security type
fpss.subscribe_full_trades("STOCK");
```
:::

## Receive Events

::: code-group
```rust [Rust]
tdx.start_streaming(|event: &FpssEvent| {
    match event {
        // --- Data events ---
        FpssEvent::Data(FpssData::Quote {
            contract_id, ms_of_day, bid, ask, bid_size, ask_size, price_type, ..
        }) => {
            let bid_price = Price::new(*bid, *price_type);
            let ask_price = Price::new(*ask, *price_type);
            println!("Quote: id={contract_id} bid={bid_price} ask={ask_price}");
        }
        FpssEvent::Data(FpssData::Trade {
            contract_id, price, size, price_type, ..
        }) => {
            let trade_price = Price::new(*price, *price_type);
            println!("Trade: id={contract_id} price={trade_price} size={size}");
        }
        FpssEvent::Data(FpssData::OpenInterest {
            contract_id, open_interest, ..
        }) => {
            println!("OI: id={contract_id} oi={open_interest}");
        }
        FpssEvent::Data(FpssData::Ohlcvc {
            contract_id, open, high, low, close, volume, count, ..
        }) => {
            println!("OHLCVC: id={contract_id} O={open} H={high} L={low} C={close}");
        }

        // --- Control events ---
        FpssEvent::Control(FpssControl::LoginSuccess { permissions }) => {
            println!("Logged in: {permissions}");
        }
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
            println!("Contract {id} assigned: {contract}");
        }
        FpssEvent::Control(FpssControl::ReqResponse { req_id, result }) => {
            println!("Request {req_id}: {:?}", result);
        }
        FpssEvent::Control(FpssControl::MarketOpen) => {
            println!("Market opened");
        }
        FpssEvent::Control(FpssControl::MarketClose) => {
            println!("Market closed");
        }
        FpssEvent::Control(FpssControl::Disconnected { reason }) => {
            println!("Disconnected: {:?}", reason);
        }
        _ => {}
    }
})?;

// Block the main thread until you want to stop
std::thread::park();
```
```python [Python]
# Track contract_id -> symbol mapping
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue  # timeout, no event

    # Control events
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
        print(f"Contract {event['id']} = {event['contract']}")
        continue

    if event["kind"] == "login_success":
        print(f"Logged in: {event['permissions']}")
        continue

    # Data events
    if event["kind"] == "quote":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Quote: {symbol} bid={event['bid']} ask={event['ask']}")

    elif event["kind"] == "trade":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Trade: {symbol} price={event['price']} size={event['size']}")

    elif event["kind"] == "open_interest":
        print(f"OI: contract={event['contract_id']} oi={event['open_interest']}")

    elif event["kind"] == "ohlcvc":
        print(f"OHLCVC: contract={event['contract_id']} "
              f"O={event['open']} H={event['high']} L={event['low']} C={event['close']}")

    elif event["kind"] == "disconnected":
        print(f"Disconnected: {event['reason']}")
        break
```
```go [Go]
for {
    event, err := fpss.NextEvent(5000) // 5s timeout
    if err != nil {
        log.Println("Error:", err)
        break
    }
    if event == nil {
        continue // timeout
    }
    fmt.Printf("Event: %s\n", string(event))
}
```
```cpp [C++]
while (true) {
    auto event = fpss.next_event(5000); // 5s timeout
    if (event.empty()) {
        continue; // timeout
    }
    std::cout << "Event: " << event << std::endl;
}
```
:::

## Contract ID Mapping

FPSS assigns integer IDs to contracts. Use `ContractAssigned` events to build a mapping from IDs to contract details.

::: code-group
```rust [Rust]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

let contracts: Arc<Mutex<HashMap<i32, Contract>>> = Arc::new(Mutex::new(HashMap::new()));
let contracts_clone = contracts.clone();

tdx.start_streaming(move |event: &FpssEvent| {
    match event {
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
            contracts_clone.lock().unwrap().insert(*id, contract.clone());
        }
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, price_type, .. }) => {
            if let Some(contract) = contracts_clone.lock().unwrap().get(contract_id) {
                let bid_price = Price::new(*bid, *price_type);
                let ask_price = Price::new(*ask, *price_type);
                println!("{}: bid={} ask={}", contract.root, bid_price, ask_price);
            }
        }
        _ => {}
    }
})?;

// Or use the built-in method:
let map: HashMap<i32, Contract> = tdx.contract_map()?;
```
```python [Python]
# Build a mapping as events arrive
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue

    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
    elif event["kind"] == "quote":
        name = contracts.get(event["contract_id"], "?")
        print(f"[QUOTE] {name}: bid={event['bid']} ask={event['ask']}")
```
```go [Go]
// Look up a contract by its server-assigned ID
contract, err := fpss.ContractLookup(42)
if err != nil {
    log.Fatal(err)
}
fmt.Println("Contract:", contract)

// List all active subscriptions
subs, _ := fpss.ActiveSubscriptions()
fmt.Println("Active:", string(subs))
```
```cpp [C++]
// Look up a contract by its server-assigned ID
auto contract = fpss.contract_lookup(42);
if (contract.has_value()) {
    std::cout << "Contract: " << contract.value() << std::endl;
}

// List all active subscriptions
auto subs = fpss.active_subscriptions();
std::cout << "Active: " << subs << std::endl;
```
:::

## Unsubscribe

::: code-group
```rust [Rust]
tdx.unsubscribe_quotes(&Contract::stock("AAPL"))?;
tdx.unsubscribe_trades(&Contract::stock("MSFT"))?;
tdx.unsubscribe_open_interest(&Contract::stock("AAPL"))?;
```
```python [Python]
tdx.unsubscribe_quotes("AAPL")
tdx.unsubscribe_trades("MSFT")
tdx.unsubscribe_open_interest("SPY")
```
```go [Go]
fpss.UnsubscribeQuotes("AAPL")
fpss.UnsubscribeTrades("MSFT")
fpss.UnsubscribeOpenInterest("AAPL")
```
```cpp [C++]
fpss.unsubscribe_quotes("AAPL");
fpss.unsubscribe_trades("MSFT");
fpss.unsubscribe_open_interest("AAPL");
```
:::

## Stop Streaming

::: code-group
```rust [Rust]
tdx.stop_streaming();
```
```python [Python]
tdx.stop_streaming()
```
```go [Go]
fpss.Shutdown()
```
```cpp [C++]
fpss.shutdown();
// RAII also handles cleanup: the FpssClient destructor calls shutdown() automatically.
```
:::

## Reconnection (Rust)

ThetaDataDx uses manual reconnection. When the server disconnects, you receive an `FpssControl::Disconnected` event with a reason code.

```rust
use thetadatadx::ThetaDataDx;
use thetadatadx::types::RemoveReason;

match thetadatadx::fpss::reconnect_delay(reason) {
    None => {
        // Permanent error (bad credentials, etc.) -- do NOT retry
        eprintln!("Permanent disconnect: {:?}", reason);
    }
    Some(delay_ms) => {
        // Wait and reconnect streaming
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        tdx.start_streaming(handler)?;
        // Re-subscribe to previous subscriptions
    }
}
```

### Disconnect Categories

| Category | Codes | Action |
|----------|-------|--------|
| Permanent | 0, 1, 2, 6, 9, 17, 18 | Do NOT reconnect |
| Rate-limited | 12 | Wait 130 seconds, then reconnect |
| Transient | All others | Wait 2 seconds, then reconnect |

## Streaming Methods Reference

### Rust (`ThetaDataDx`)

| Method | Description |
|--------|-------------|
| `start_streaming(callback)` | Begin streaming with an event callback |
| `subscribe_quotes(contract)` | Subscribe to quote data |
| `subscribe_trades(contract)` | Subscribe to trade data |
| `subscribe_open_interest(contract)` | Subscribe to open interest |
| `subscribe_full_trades(sec_type)` | Subscribe to all trades for a security type |
| `unsubscribe_quotes(contract)` | Unsubscribe from quotes |
| `unsubscribe_trades(contract)` | Unsubscribe from trades |
| `unsubscribe_open_interest(contract)` | Unsubscribe from OI |
| `contract_map()` | Get current contract ID mapping |
| `stop_streaming()` | Stop the streaming connection |

### Python (`ThetaDataDx`)

| Method | Description |
|--------|-------------|
| `start_streaming()` | Connect to FPSS streaming servers |
| `subscribe_quotes(symbol)` | Subscribe to quote data |
| `subscribe_trades(symbol)` | Subscribe to trade data |
| `subscribe_open_interest(symbol)` | Subscribe to open interest |
| `next_event(timeout_ms=5000)` | Poll next event (dict or `None`) |
| `stop_streaming()` | Graceful shutdown of streaming |

### Go (`FpssClient`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `SubscribeQuotes` | `(symbol string) (int, error)` | Subscribe to quotes |
| `SubscribeTrades` | `(symbol string) (int, error)` | Subscribe to trades |
| `SubscribeOpenInterest` | `(symbol string) (int, error)` | Subscribe to OI |
| `SubscribeFullTrades` | `(secType string) (int, error)` | Subscribe to all trades for a security type |
| `UnsubscribeQuotes` | `(symbol string) (int, error)` | Unsubscribe from quotes |
| `UnsubscribeTrades` | `(symbol string) (int, error)` | Unsubscribe from trades |
| `UnsubscribeOpenInterest` | `(symbol string) (int, error)` | Unsubscribe from OI |
| `NextEvent` | `(timeoutMs uint64) (json.RawMessage, error)` | Poll next event |
| `IsAuthenticated` | `() bool` | Check FPSS auth status |
| `ContractLookup` | `(id int) (string, error)` | Look up contract by server-assigned ID |
| `ActiveSubscriptions` | `() (json.RawMessage, error)` | Get active subscriptions |
| `Shutdown` | `()` | Graceful shutdown |

### C++ (`FpssClient`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `subscribe_quotes` | `(symbol) -> int32_t` | Subscribe to quotes |
| `subscribe_trades` | `(symbol) -> int32_t` | Subscribe to trades |
| `subscribe_open_interest` | `(symbol) -> int32_t` | Subscribe to OI |
| `subscribe_full_trades` | `(sec_type) -> int32_t` | Subscribe to all trades for a security type |
| `unsubscribe_trades` | `(symbol) -> int32_t` | Unsubscribe from trades |
| `unsubscribe_open_interest` | `(symbol) -> int32_t` | Unsubscribe from OI |
| `next_event` | `(timeout_ms) -> std::string` | Poll next event (empty on timeout) |
| `is_authenticated` | `() -> bool` | Check FPSS auth status |
| `contract_lookup` | `(id) -> std::optional<std::string>` | Look up contract by server-assigned ID |
| `active_subscriptions` | `() -> std::string` | Get active subscriptions as JSON |
| `shutdown` | `() -> void` | Graceful shutdown |

## Event Reference

### Data Events

| Event | Key Fields |
|-------|------------|
| `Quote` | contract_id, ms_of_day, bid, ask, bid_size, ask_size, price_type, date |
| `Trade` | contract_id, ms_of_day, price, size, exchange, condition, price_type, date |
| `OpenInterest` | contract_id, ms_of_day, open_interest, date |
| `Ohlcvc` | contract_id, ms_of_day, open, high, low, close, volume, count, price_type, date |

### Control Events

| Event | Fields |
|-------|--------|
| `LoginSuccess` | permissions (string) |
| `ContractAssigned` | id, contract |
| `ReqResponse` | req_id, result (Subscribed/Error/MaxStreamsReached/InvalidPerms) |
| `MarketOpen` | (none) |
| `MarketClose` | (none) |
| `ServerError` | message |
| `Disconnected` | reason (RemoveReason enum) |
| `Error` | message |

## Complete Example

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let contracts: Arc<Mutex<HashMap<i32, Contract>>> = Arc::new(Mutex::new(HashMap::new()));
    let contracts_clone = contracts.clone();

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                contracts_clone.lock().unwrap().insert(*id, contract.clone());
            }
            FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, price_type, .. }) => {
                if let Some(c) = contracts_clone.lock().unwrap().get(contract_id) {
                    let bid_p = Price::new(*bid, *price_type);
                    let ask_p = Price::new(*ask, *price_type);
                    println!("[QUOTE] {}: bid={} ask={}", c.root, bid_p, ask_p);
                }
            }
            FpssEvent::Data(FpssData::Trade { contract_id, price, size, price_type, .. }) => {
                if let Some(c) = contracts_clone.lock().unwrap().get(contract_id) {
                    let trade_p = Price::new(*price, *price_type);
                    println!("[TRADE] {}: price={} size={}", c.root, trade_p, size);
                }
            }
            FpssEvent::Control(FpssControl::Disconnected { reason }) => {
                eprintln!("Disconnected: {:?}", reason);
            }
            _ => {}
        }
    })?;

    tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
    tdx.subscribe_trades(&Contract::stock("AAPL"))?;
    tdx.subscribe_quotes(&Contract::stock("MSFT"))?;

    // Block until interrupted
    std::thread::park();
    tdx.stop_streaming();
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx
import signal
import sys

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Start streaming
tdx.start_streaming()

# Graceful shutdown on Ctrl+C
def shutdown_handler(sig, frame):
    tdx.stop_streaming()
    sys.exit(0)

signal.signal(signal.SIGINT, shutdown_handler)

# Subscribe to multiple streams
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("AAPL")
tdx.subscribe_quotes("MSFT")

contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue

    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
    elif event["kind"] == "quote":
        name = contracts.get(event["contract_id"], "?")
        print(f"[QUOTE] {name}: bid={event['bid']} ask={event['ask']}")
    elif event["kind"] == "trade":
        name = contracts.get(event["contract_id"], "?")
        print(f"[TRADE] {name}: price={event['price']} size={event['size']}")
    elif event["kind"] == "disconnected":
        print(f"Disconnected: {event['reason']}")
        break

tdx.stop_streaming()
```
```go [Go]
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/ThetaDataDx/sdks/go"
)

func main() {
    creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    // Historical client
    client, err := thetadatadx.Connect(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Streaming client (separate connection, same credentials)
    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Close()

    // Subscribe to real-time data
    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeTrades("AAPL")

    // Process events
    for {
        event, err := fpss.NextEvent(5000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue
        }
        fmt.Printf("Event: %s\n", string(event))
    }

    fpss.Shutdown()
}
```
```cpp [C++]
#include "thetadatadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    // Historical client
    auto client = tdx::Client::connect(creds, config);

    // Streaming client (separate connection, same credentials)
    tdx::FpssClient fpss(creds, config);

    // Subscribe to quotes and trades
    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_trades("AAPL");
    fpss.subscribe_trades("MSFT");

    // Process events
    while (true) {
        auto event = fpss.next_event(5000);
        if (event.empty()) {
            continue;
        }
        std::cout << "Event: " << event << std::endl;
    }

    fpss.shutdown();
}
```
:::
