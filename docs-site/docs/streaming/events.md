---
title: Handling Events
description: The typed event catalogue and how to narrow events in each language.
---

# Handling Events

Every streaming update reaches your callback as one typed event. There are five **data** events and a set of **control** events covering session lifecycle; your callback narrows on the event type and reads typed fields — no JSON, no string parsing.

## Narrowing

<SdkTabs>

<template #rust>

```rust
use thetadatadx::fpss::{StreamControl, StreamData, StreamEvent};

client.stream().start_streaming(|event: &StreamEvent| {
    match event {
        StreamEvent::Data(StreamData::Quote { contract, bid, ask, .. }) => {
            println!("{} bid={bid} ask={ask}", contract.symbol);
        }
        StreamEvent::Data(StreamData::Trade { contract, price, size, .. }) => {
            println!("{} {price} x {size}", contract.symbol);
        }
        StreamEvent::Control(StreamControl::Disconnected { .. }) => {
            eprintln!("disconnected; automatic reconnect underway");
        }
        _ => {}
    }
})?;
```

</template>

<template #python>

```python
def on_event(event):
    # Every event is a typed class with a snake_case `kind` discriminator.
    if event.kind == "quote":
        print(event.contract.symbol, event.bid, event.ask)
    elif event.kind == "trade":
        print(event.contract.symbol, event.price, event.size)
    elif event.kind == "disconnected":
        print("disconnected; automatic reconnect underway")

client.stream.start_streaming(on_event)
```

`match event: case Quote(): ...` works too — the event classes (`Quote`, `Trade`, `OpenInterest`, `Ohlcvc`, …) are importable from `thetadatadx`.

</template>

<template #typescript>

```typescript
await client.stream.startStreaming((event) => {
  switch (event.kind) {
    case 'quote': {
      const q = event.quote!;
      console.log(q.contract.symbol, q.bid, q.ask);
      break;
    }
    case 'trade': {
      const t = event.trade!;
      console.log(t.contract.symbol, t.price, t.size);
      break;
    }
    case 'disconnected':
      console.warn('disconnected; automatic reconnect underway');
      break;
  }
});
```

`event.kind` is a literal union, so the `switch` narrows the matching payload field (`event.quote`, `event.trade`, …).

</template>

<template #cpp>

```cpp
client.stream().set_callback([](const thetadatadx::StreamEvent& event) {
    switch (event.kind) {
    case THETADATADX_FPSS_QUOTE: {
        auto& q = event.quote;
        std::cout << q.contract.symbol << " bid=" << q.bid << " ask=" << q.ask << "\n";
        break;
    }
    case THETADATADX_FPSS_TRADE: {
        auto& t = event.trade;
        std::cout << t.contract.symbol << " " << t.price << " x " << t.size << "\n";
        break;
    }
    case THETADATADX_FPSS_DISCONNECTED:
        std::cerr << "disconnected; automatic reconnect underway\n";
        break;
    default:
        break;
    }
});
```

</template>

<template #http>

```bash
websocat ws://127.0.0.1:25520/v1/events
```

Over the [server's WebSocket](/server/websocket), each event arrives as one JSON message with a `header.type` of `QUOTE`, `TRADE`, `OHLC`, `OPEN_INTEREST`, or `STATUS`.

</template>

</SdkTabs>

## Data events

| Kind | Fields | Delivered for |
|---|---|---|
| `quote` | NBBO sides: `bid` / `ask` price, size, exchange, condition; `ms_of_day`, `date`, `received_at_ns` | [Quote streams](/streaming/stocks/quote) |
| `trade` | `price`, `size`, `exchange`, `condition` (+ extended conditions), `sequence`, flags, `ms_of_day`, `date`, `received_at_ns` | [Trade](/streaming/stocks/trade), [full-trade](/streaming/options/full-trade), and [index price](/streaming/indices/price) streams |
| `open_interest` | `open_interest`, `ms_of_day`, `date`, `received_at_ns` | [Open-interest streams](/streaming/options/open-interest) |
| `ohlcvc` | `open`, `high`, `low`, `close`, `volume`, `count`, `ms_of_day`, `date`, `received_at_ns` | Bars derived from any active trade subscription |
| `market_value` | `market_price` (calculated value), `market_bid` / `market_ask` (stocks and options only), `ms_of_day`, `date`, `received_at_ns` | [Index market value](/streaming/indices/market-value) and per-contract market-value streams |

Each stream-type page in the sidebar lists its event's complete field table.

### The contract on every data event

Data events carry a resolved, typed contract — read identity straight off the event, no lookup table:

| Field | Type | Notes |
|---|---|---|
| `symbol` | string | Underlying or ticker. |
| `sec_type` | string | `STOCK` / `OPTION` / `INDEX` / `RATE`. |
| `expiration` | int? | `YYYYMMDD`; options only. |
| `right` | string? | `C` / `P`; options only. |
| `strike` | float? | Strike in dollars; options only — the same unit historical rows carry under the same name (Rust exposes `strike_dollars()` over the codec integer; the C ABI field is dollars). |

## Control events

Lifecycle and session events share the callback. The ones worth handling:

| Kind | Meaning |
|---|---|
| `connected` / `login_success` | Session is up and authenticated. |
| `contract_assigned` | The server bound a contract to the session (informational). |
| `req_response` | Acknowledgement for a subscribe/unsubscribe, with its request id. |
| `market_open` / `market_close` | Session boundary notices. |
| `disconnected` / `reconnecting` / `reconnected` | Automatic-reconnect progress; see [Reconnection & Monitoring](/streaming/reliability). |
| `reconnects_exhausted` | The retry budget is spent; the session is down for good until you intervene. |
| `server_error` / `parse_error` | Server-reported (`ServerError`) or protocol-level parse (`ParseError`) error with a message payload. |
| `ping`, `restart`, `reconnected_server`, `unknown_frame`, `unknown_control` | Heartbeats, server restarts, and forward-compatibility fallbacks. |

Unrecognized future event types arrive as `unknown_*` rather than being dropped, so a callback written today keeps working against newer servers.
