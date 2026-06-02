---
title: Reconnection & Error Handling
description: Handle streaming disconnects, implement reconnection logic with reconnect_streaming() or reconnect(), and manage streaming errors.
---

# Reconnection & Error Handling

## Reconnection APIs

Rust exposes `reconnect_streaming(handler)` on the unified `ThetaDataDxClient` client.
Python, TypeScript/Node.js, and C++ expose `reconnect()` on their public streaming clients.

## Reconnection with `reconnect_streaming()` (Rust)

The unified `ThetaDataDxClient` client provides `reconnect_streaming()` which handles the full reconnection cycle automatically:

1. Saves all active per-contract and full-stream subscriptions
2. Stops the current streaming connection
3. Starts a new streaming connection with your handler
4. Re-subscribes everything that was previously active

```rust
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
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
                FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
                    println!("Quote: {} {bid:.2}/{ask:.2}", contract.symbol);
                }
                _ => {}
            }
        })?;
    }
}
```

::: tip
`reconnect_streaming()` uses the same `DirectConfig` (including `fpss_hosts`) that was passed at `ThetaDataDxClient::connect()` time. If hosts change, create a new `ThetaDataDxClient` instance.
:::

## Reconnection with `reconnect()` (Python, C++)

::: code-group
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

def on_event(event):
    print(event)

tdx.start_streaming(on_event)
tdx.subscribe(Contract.stock("AAPL").quote())
tdx.subscribe(Contract.option(
    "SPY", expiration="20260116", strike="600", right="C"
).quote())

# reconnect() restores the existing subscription set; the callback
# registered above is reused on the new session.
tdx.reconnect()
```
```cpp [C++]
#include "thetadx.hpp"

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    // FpssClient owns the streaming + reconnect surface.
    tdx::FpssClient fpss(creds, config);
    fpss.subscribe(tdx::Contract::stock("AAPL").quote());
    fpss.subscribe(tdx::Contract::option(
        "SPY", "20260116", "600", "C"
    ).quote());

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
    config.fpss_ring_size,     // ring buffer size
    config.fpss_flush_mode,    // Batched or Immediate
    handler,                   // FnMut(&FpssEvent)
)?;
```

Waits the specified delay, connects to a new streaming server, and re-subscribes all previous subscriptions with `req_id = -1`.

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
ThetaDataDx treats all 7 credential/account error codes as permanent. No amount of retrying will fix bad credentials.
:::

### Rate-Limited Disconnect

Code 12 indicates you have exceeded the connection rate limit. Wait the full 130 seconds before attempting to reconnect, or the cooldown resets.

### Transient Disconnects

All other codes indicate temporary issues (network glitch, server restart, etc.). A 2-second delay before reconnection is sufficient.

## Complete Example with Reconnection

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Data(FpssData::Quote {
                contract, bid, ask, received_at_ns, ..
            }) => {
                println!("[QUOTE] {}: bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns",
                    contract.symbol);
            }
            FpssEvent::Data(FpssData::Trade {
                contract, price, size, received_at_ns, ..
            }) => {
                println!("[TRADE] {}: price={price:.2} size={size} rx={received_at_ns}ns",
                    contract.symbol);
            }
            FpssEvent::Control(FpssControl::Disconnected { reason }) => {
                eprintln!("Disconnected: {:?}", reason);
                // Handle reconnection in your outer loop
            }
            _ => {}
        }
    })?;

    tdx.subscribe(Contract::stock("AAPL").quote())?;
    tdx.subscribe(Contract::stock("AAPL").trade())?;
    tdx.subscribe(Contract::stock("MSFT").quote())?;

    // Block until interrupted
    std::thread::park();
    tdx.stop_streaming();
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract
import signal
import sys

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

# Graceful shutdown on Ctrl+C
def shutdown_handler(sig, frame):
    tdx.stop_streaming()
    sys.exit(0)

signal.signal(signal.SIGINT, shutdown_handler)

# Push-callback delivery via the `streaming(callback)` context
# manager. The `with` block pairs `stop_streaming()` + `await_drain()`
# on exit so the consumer thread has finished firing `on_event`
# before the scope returns.
def on_event(event):
    if event.kind == "quote":
        print(f"[QUOTE] {event.contract.symbol}: bid={event.bid} ask={event.ask} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "trade":
        print(f"[TRADE] {event.contract.symbol}: price={event.price} size={event.size} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "disconnected":
        print(f"Disconnected: reason={event.reason}")

with tdx.streaming(on_event):
    tdx.subscribe(Contract.stock("AAPL").quote())
    tdx.subscribe(Contract.stock("AAPL").trade())
    tdx.subscribe(Contract.stock("MSFT").quote())
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

    // Typed control variants — one C struct per FpssControl::*
    // Rust variant. Dispatch on event.kind, read the matching
    // event.<variant> payload.
    client.set_callback([](const tdx::FpssEvent& event) {
        switch (event.kind) {
        case TDX_FPSS_QUOTE: {
            auto& q = event.quote;
            std::cout << "[QUOTE] " << q.contract.symbol
                      << " bid=" << q.bid << " ask=" << q.ask
                      << " rx=" << q.received_at_ns << "ns" << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            auto& t = event.trade;
            std::cout << "[TRADE] " << t.contract.symbol
                      << " price=" << t.price << " size=" << t.size << std::endl;
            break;
        }
        case TDX_FPSS_DISCONNECTED:
            std::cout << "Disconnected: reason=" << event.disconnected.reason
                      << std::endl;
            break;
        default:
            break;
        }
    });

    // Subscribe via the unified contract-first API.
    client.subscribe(tdx::Contract::stock("AAPL").quote());
    client.subscribe(tdx::Contract::stock("AAPL").trade());
    client.subscribe(tdx::Contract::stock("MSFT").trade());

    // ... let the callback run ...
    client.stop_streaming();
}
```
:::
