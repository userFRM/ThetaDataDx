---
title: Streaming — Getting Started
description: Connect, register a callback, subscribe, and shut down cleanly — in every language.
---

# Streaming

Real-time quotes, trades, and open interest are delivered as **typed events through a callback you register once**. The same client that serves historical requests runs the streaming session: connect, start streaming, subscribe.

Streaming requires a Standard subscription or higher on the matching asset class — see [Subscriptions](/articles/subscriptions). Markets closed? Connect with the `dev()` [configuration](/articles/configuration) to stream a replayed session.

## Connect, subscribe, receive

<SdkTabs>

<template #rust>

```rust
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::{Credentials, DirectConfig, ThetaDataDxClient};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    tdx.start_streaming(|event: &FpssEvent| {
        if let FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) = event {
            println!("{} bid={bid} ask={ask}", contract.symbol);
        }
    })?;

    tdx.subscribe(Contract::stock("AAPL").quote())?;
    std::thread::sleep(std::time::Duration::from_secs(60));

    tdx.stop_streaming();
    Ok(())
}
```

The callback runs on a dedicated consumer thread — no async executor between the wire and your code. `subscribe` / `unsubscribe` are callable from any thread.

</template>

<template #python>

```python
import time

from thetadatadx import Config, Contract, Credentials, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

def on_event(event):
    if event.kind == "quote":
        print(event.contract.symbol, event.bid, event.ask)

with tdx.streaming(on_event):          # stops streaming and drains on exit
    tdx.subscribe(Contract.stock("AAPL").quote())
    time.sleep(60)
```

Prefer the `with tdx.streaming(...)` context manager; it pairs `stop_streaming()` with a drain wait on exit. `tdx.start_streaming(on_event)` / `tdx.stop_streaming()` are the explicit form.

</template>

<template #typescript>

```typescript
import { Contract, ThetaDataDxClient } from 'thetadatadx';

const tdx = ThetaDataDxClient.connectFromFile('creds.txt');

tdx.startStreaming((event) => {
  if (event.kind === 'quote') {
    const q = event.quote!;
    console.log(q.contract.symbol, q.bid, q.ask);
  }
});

tdx.subscribe(Contract.stock('AAPL').quote());

setTimeout(() => tdx.stopStreaming(), 60_000);
```

</template>

<template #cpp>

```cpp
#include "thetadx.hpp"
#include <iostream>
#include <thread>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::UnifiedClient::connect(creds, tdx::Config::production());

    client.set_callback([](const tdx::FpssEvent& event) {
        if (event.kind == TDX_FPSS_QUOTE) {
            auto& q = event.quote;
            std::cout << q.contract.symbol << " bid=" << q.bid << " ask=" << q.ask << "\n";
        }
    });

    client.subscribe(tdx::Contract::stock("AAPL").quote());
    std::this_thread::sleep_for(std::chrono::seconds(60));
    // RAII: the destructor stops streaming and drains.
}
```

</template>

<template #http>

```bash
thetadatadx-server --creds creds.txt &

websocat ws://127.0.0.1:25520/v1/events
{"msg_type": "STREAM", "sec_type": "STOCK", "req_type": "QUOTE", "id": 1, "add": true, "contract": {"symbol": "AAPL"}}
```

The [server binary](/server/) bridges the stream onto a local WebSocket — see [WebSocket Streaming](/server/websocket) for the envelope and event formats.

</template>

</SdkTabs>

## Subscriptions

Build a typed subscription from a `Contract` (per-contract scope) or a `SecType` (full-stream scope), then pass it to `subscribe` / `unsubscribe`:

- Per contract: `Contract.stock("AAPL")`, `Contract.option("SPY", "20260618", "570", "C")`, `Contract.index("SPX")` — then `.quote()`, `.trade()`, or `.open_interest()`.
- Full stream (every contract of a security type, stocks and options only): `SecType` + `.full_trades()` / `.full_open_interest()`.
- `subscribe_many([...])` installs a batch in one call; `active_subscriptions()` snapshots what is installed.

The per-stream-type pages in the sidebar carry the exact subscribe code, the event fields, and the unsubscribe call for each stream.

## Lifecycle

- **Start once, subscribe many.** `start_streaming(callback)` opens the session; subscriptions attach and detach freely afterwards.
- **Stopping.** `stop_streaming()` closes the session and clears the callback (in C++, the destructor does this). `await_drain(timeout_ms)` blocks until queued events have been delivered.
- **Reconnects are automatic** with resubscription of everything you had installed; policy and monitoring live in [Reconnection & Monitoring](/streaming/reliability).
- **Event order is per-connection arrival order;** every data event carries `received_at_ns`, the local receive timestamp.
