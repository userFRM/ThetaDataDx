---
title: Reconnection & Error Handling
description: Handle FPSS disconnects, implement reconnection logic with reconnect_streaming() or reconnect(), and manage streaming errors.
---

# Reconnection & Error Handling

## Reconnection APIs

Rust exposes `reconnect_streaming(handler)` on the unified `ThetaDataDx` client.
Python, TypeScript/Node.js, Go, and C++ expose `reconnect()` on their public streaming clients.

## Reconnection with `reconnect_streaming()` (Rust)

The unified `ThetaDataDx` client provides `reconnect_streaming()` which handles the full reconnection cycle automatically:

1. Saves all active per-contract and firehose subscriptions
2. Stops the current streaming connection
3. Starts a new streaming connection with your handler
4. Re-subscribes everything that was previously active

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use tdbe::types::enums::RemoveReason;

// When you detect a disconnect, reconnect with a new handler:
match thetadatadx::fpss::reconnect_delay(reason) {
    None => {
        // Permanent error (bad credentials, etc.) -- do NOT retry
        eprintln!("Permanent disconnect: {:?}", reason);
    }
    Some(delay_ms) => {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        // reconnect_streaming() saves subs, stops, reconnects, and re-subscribes
        tdx.reconnect_streaming(|event: &FpssEvent| {
            // Your event handler -- same signature as start_streaming()
            match event {
                FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
                    println!("Quote: {contract_id} {bid:.2}/{ask:.2}");
                }
                _ => {}
            }
        })?;
    }
}
```

::: tip
`reconnect_streaming()` uses the same `DirectConfig` (including `fpss_hosts`) that was passed at `ThetaDataDx::connect()` time. If hosts change, create a new `ThetaDataDx` instance.
:::

## Reconnection with `reconnect()` (Python, Go, C++)

::: code-group
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
tdx.subscribe_option_quotes("SPY", "20260116", "600", "C")

# reconnect() restores the existing subscription set
tdx.reconnect()
```
```go [Go]
package main

import (
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Close()

    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeOptionQuotes("SPY", "20260116", "600", "C")

    if err := fpss.Reconnect(); err != nil {
        log.Fatal(err)
    }
}
```
```cpp [C++]
#include "thetadx.hpp"

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    tdx::FpssClient fpss(creds, config);
    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_option_quotes("SPY", "20260116", "600", "C");

    fpss.reconnect();
}
```
:::

## Manual Reconnection (Low-Level Rust)

For fine-grained control, use the low-level `fpss::reconnect()` function directly:

```rust
use thetadatadx::fpss;
use thetadatadx::config::FpssFlushMode;

let new_client = fpss::reconnect(
    &creds,
    &config.fpss_hosts,       // hosts to connect to
    previous_subs,             // Vec<(SubscriptionKind, Contract)>
    previous_full_subs,        // Vec<(SubscriptionKind, SecType)>
    delay_ms,                  // reconnection delay
    config.fpss_ring_size,     // Disruptor ring size
    config.fpss_flush_mode,    // Batched or Immediate
    handler,                   // FnMut(&FpssEvent)
)?;
```

This is the Rust equivalent of Java's `FPSSClient.handleInvoluntaryDisconnect()`. It waits the specified delay, connects to a new server, and re-subscribes all previous subscriptions with `req_id = -1`.

## `reconnect_delay()`

The `fpss::reconnect_delay()` helper classifies disconnect reasons and returns the appropriate delay:

```rust
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64>
```

- Returns `None` for permanent errors (do not reconnect)
- Returns `Some(130_000)` for rate-limited disconnects (130 seconds)
- Returns `Some(2_000)` for transient disconnects (2 seconds)

## Disconnect Categories

| Category | Codes | Delay | Action |
|----------|-------|-------|--------|
| **Permanent** | 0, 1, 2, 6, 9, 17, 18 | -- | Do NOT reconnect. Bad credentials, suspended account, or server-side permanent error. |
| **Rate-limited** | 12 (TooManyRequests) | 130 seconds | Wait the full cooldown or it resets. |
| **Transient** | All others | 2 seconds | Network glitch, server restart, etc. |

### Permanent Disconnect Reasons

Permanent disconnects indicate a problem that will not resolve by retrying:

- **Code 0, 1, 2** -- Authentication failures (bad credentials, expired subscription)
- **Code 6** -- Account suspended
- **Code 9** -- Invalid request parameters
- **Code 17, 18** -- Server-side permanent errors

::: warning
Unlike the Java terminal (which only treats `AccountAlreadyConnected` as permanent), ThetaDataDx treats all 7 credential/account error codes as permanent. No amount of retrying will fix bad credentials.
:::

### Rate-Limited Disconnect

Code 12 indicates you have exceeded the connection rate limit. Wait the full 130 seconds before attempting to reconnect, or the cooldown resets.

### Transient Disconnects

All other codes indicate temporary issues (network glitch, server restart, etc.). A 2-second delay before reconnection is sufficient.

## Complete Example with Reconnection

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
            FpssEvent::Data(FpssData::Quote {
                contract_id, bid, ask, received_at_ns, ..
            }) => {
                if let Some(c) = contracts_clone.lock().unwrap().get(contract_id) {
                    println!("[QUOTE] {}: bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns",
                        c.root);
                }
            }
            FpssEvent::Data(FpssData::Trade {
                contract_id, price, size, received_at_ns, ..
            }) => {
                if let Some(c) = contracts_clone.lock().unwrap().get(contract_id) {
                    println!("[TRADE] {}: price={price:.2} size={size} rx={received_at_ns}ns",
                        c.root);
                }
            }
            FpssEvent::Control(FpssControl::Disconnected { reason }) => {
                eprintln!("Disconnected: {:?}", reason);
                // Handle reconnection in your outer loop
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

    # Control events flatten into `Simple` pyclass — branch on
    # `event.kind == "simple"` then inspect `event.event_type`.
    if event.kind == "simple" and event.event_type == "contract_assigned":
        # event.id -> contract_id, event.detail -> formatted contract string
        contracts[event.id] = event.detail
    elif event.kind == "quote":
        name = contracts.get(event.contract_id, "?")
        print(f"[QUOTE] {name}: bid={event.bid} ask={event.ask} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "trade":
        name = contracts.get(event.contract_id, "?")
        print(f"[TRADE] {name}: price={event.price} size={event.size} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "simple" and event.event_type == "disconnected":
        print(f"Disconnected: {event.detail}")
        break

tdx.stop_streaming()
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
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Close()

    // Subscribe to real-time data
    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeTrades("AAPL")

    // Process typed events
    for {
        event, err := fpss.NextEvent(5000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue
        }

        switch event.Kind {
        case thetadatadx.FpssQuoteEvent:
            q := event.Quote
            // Bid and Ask are pre-decoded to float64
            fmt.Printf("[QUOTE] contract=%d bid=%.4f ask=%.4f rx=%dns\n",
                q.ContractID, q.Bid, q.Ask, q.ReceivedAtNs)

        case thetadatadx.FpssTradeEvent:
            t := event.Trade
            // Price is pre-decoded to float64
            fmt.Printf("[TRADE] contract=%d price=%.4f size=%d\n",
                t.ContractID, t.Price, t.Size)

        case thetadatadx.FpssControlEvent:
            ctrl := event.Control
            if ctrl.Kind == 6 { // Disconnected
                fmt.Printf("Disconnected: %s\n", ctrl.Detail)
            }
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

    // Subscribe to quotes and trades
    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_trades("AAPL");
    fpss.subscribe_trades("MSFT");

    // Process typed events
    while (true) {
        auto event = fpss.next_event(5000);
        if (!event) {
            continue;
        }

        switch (event->kind) {
        case TDX_FPSS_QUOTE: {
            auto& q = event->quote;
            
            
            std::cout << "[QUOTE] contract=" << q.contract_id
                      << " bid=" << q.bid << " ask=" << q.ask
                      << " rx=" << q.received_at_ns << "ns" << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            auto& t = event->trade;
            
            std::cout << "[TRADE] contract=" << t.contract_id
                      << " price=" << t.price << " size=" << t.size << std::endl;
            break;
        }
        case TDX_FPSS_CONTROL: {
            auto& c = event->control;
            if (c.kind == 6) { // Disconnected
                std::cout << "Disconnected";
                if (c.detail) std::cout << ": " << c.detail;
                std::cout << std::endl;
            }
            break;
        }
        default:
            break;
        }
    }

    fpss.shutdown();
}
```
:::
