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
use thetadatadx::streaming::Contract;
use thetadatadx::streaming::{StreamData, StreamEvent};
use thetadatadx::{Credentials, DirectConfig, Client};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = Client::connect(&creds, DirectConfig::production()).await?;

    client.stream().start_streaming(|event: &StreamEvent| {
        if let StreamEvent::Data(StreamData::Quote { contract, bid, ask, .. }) = event {
            println!("{} bid={bid} ask={ask}", contract.symbol);
        }
    })?;

    client.stream().subscribe(Contract::stock("AAPL").quote())?;
    std::thread::sleep(std::time::Duration::from_secs(60));

    client.stream().stop_streaming();
    Ok(())
}
```

The callback runs on a dedicated consumer thread — no async executor between the wire and your code. `subscribe` / `unsubscribe` are callable from any thread.

</template>

<template #python>

```python
import time

from thetadatadx import Config, Contract, Credentials, Client

creds = Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

def on_event(event):
    if event.kind == "quote":
        print(event.contract.symbol, event.bid, event.ask)

with client.streaming(on_event):          # stops streaming and drains on exit
    client.stream.subscribe(Contract.stock("AAPL").quote())
    time.sleep(60)
```

Prefer the `with client.streaming(...)` context manager; it pairs `stop_streaming()` with a drain wait on exit. `client.stream.start_streaming(on_event)` / `client.stream.stop_streaming()` are the explicit form.

</template>

<template #typescript>

```typescript
import { Contract, Client } from 'thetadatadx';

const client = await Client.connectFromFile('creds.txt');

await client.stream.startStreaming((event) => {
  if (event.kind === 'quote') {
    const q = event.quote!;
    console.log(q.contract.symbol, q.bid, q.ask);
  }
});

client.stream.subscribe(Contract.stock('AAPL').quote());

setTimeout(() => client.stream.stopStreaming(), 60_000);
```

</template>

<template #cpp>

```cpp
#include "thetadatadx.hpp"
#include <iostream>
#include <thread>

int main() {
    auto creds = thetadatadx::Credentials::from_file("creds.txt");
    auto client = thetadatadx::Client::connect(creds, thetadatadx::Config::production());

    client.stream().set_callback([](const thetadatadx::StreamEvent& event) {
        if (event.kind == THETADATADX_FPSS_QUOTE) {
            auto& q = event.quote;
            std::cout << q.contract.symbol << " bid=" << q.bid << " ask=" << q.ask << "\n";
        }
    });

    client.stream().subscribe(thetadatadx::Contract::stock("AAPL").quote());
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

- Per contract: `Contract.stock("AAPL")`, `Contract.option("SPY", { expiration: "20260618", strike: "570", right: "C" })`, `Contract.index("SPX")` — then `.quote()`, `.trade()`, or `.open_interest()`. The option leg is named (a keyword argument in Python, an options object in TypeScript, a struct in Rust / C++) so a swapped expiration/strike/right cannot pass silently.
- Full stream (every contract of a security type, stocks and options only): `SecType` + `.full_trades()` / `.full_open_interest()`.
- `subscribe_many([...])` installs a batch in one call; `active_subscriptions()` snapshots what is installed.

The per-stream-type pages in the sidebar carry the exact subscribe code, the event fields, and the unsubscribe call for each stream.

## Lifecycle

- **Start once, subscribe many.** `client.stream.start_streaming(callback)` opens the session; subscriptions attach and detach freely afterwards.
- **Stopping.** `client.stream.stop_streaming()` closes the session and clears the callback (in C++, the destructor does this). `client.stream.await_drain(timeout_ms)` blocks until queued events have been delivered.
- **Reconnects are automatic** with resubscription of everything you had installed; policy and monitoring live in [Reconnection & Monitoring](/streaming/reliability).
- **Event order is per-connection arrival order;** every data event carries `received_at_ns`, the local receive timestamp.
