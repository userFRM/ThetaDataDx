---
title: Reconnection & Error Handling
description: Handle FPSS disconnects, implement reconnection logic, and manage streaming errors.
---

# Reconnection & Error Handling

## Reconnection (Rust)

ThetaDataDx uses manual reconnection. When the server disconnects, you receive an `FpssControl::Disconnected` event with a reason code.

```rust
use thetadatadx::ThetaDataDx;
use thetadatadx::types::RemoveReason;

match thetadatadx::fpss::reconnect_delay(reason) {
    None => {
        // Permanent error (bad credentials, etc.) - do NOT retry
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

::: warning
After reconnection, you must re-subscribe to all previously active streams. The server does not remember your subscriptions across connections.
:::

## Disconnect Categories

| Category | Codes | Action |
|----------|-------|--------|
| Permanent | 0, 1, 2, 6, 9, 17, 18 | Do NOT reconnect |
| Rate-limited | 12 | Wait 130 seconds, then reconnect |
| Transient | All others | Wait 2 seconds, then reconnect |

### Permanent Disconnect Reasons

Permanent disconnects indicate a problem that will not resolve by retrying:

- **Code 0, 1, 2** - Authentication failures (bad credentials, expired subscription)
- **Code 6** - Account suspended
- **Code 9** - Invalid request parameters
- **Code 17, 18** - Server-side permanent errors

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
