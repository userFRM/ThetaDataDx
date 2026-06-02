---
title: Handling Events
description: Process data and control events from the streaming connection - quotes, trades, open interest, OHLCVC, control messages, and raw data.
---

# Handling Events

## Receive Events

::: code-group
```rust [Rust]
client.start_streaming(|event: &FpssEvent| {
    match event {
        // --- Data events ---
        // Each data variant carries an `Arc<Contract>`, so `contract.symbol`
        // (plus `.expiration` / `.strike` / `.right()` on options) is readable
        // inline — no contract-ID map lookup required.
        FpssEvent::Data(FpssData::Quote {
            contract, ms_of_day, bid, ask, bid_size, ask_size,
            received_at_ns, ..
        }) => {
            println!("Quote: {} bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns", contract.symbol);
        }
        FpssEvent::Data(FpssData::Trade {
            contract, price, size, sequence, received_at_ns, ..
        }) => {
            println!("Trade: {} price={price:.2} size={size} seq={sequence}", contract.symbol);
        }
        FpssEvent::Data(FpssData::OpenInterest {
            contract, open_interest, received_at_ns, ..
        }) => {
            println!("OI: {} oi={open_interest} rx={received_at_ns}ns", contract.symbol);
        }
        FpssEvent::Data(FpssData::Ohlcvc {
            contract, open, high, low, close,
            volume, count, received_at_ns, ..
        }) => {
            // volume and count are i64 to avoid overflow on high-volume symbols
            println!("OHLCVC: {} O={open:.2} H={high:.2} L={low:.2} C={close:.2} vol={volume} n={count}", contract.symbol);
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
        FpssEvent::Control(FpssControl::ServerError { message }) => {
            eprintln!("Server error: {message}");
        }
        FpssEvent::Control(FpssControl::Disconnected { reason }) => {
            eprintln!("Disconnected: {:?}", reason);
        }
        FpssEvent::Control(FpssControl::Error { message }) => {
            eprintln!("Error: {message}");
        }

        // --- Unrecognised wire-frame fallback ---
        FpssEvent::Control(FpssControl::UnknownFrame { code, payload }) => {
            eprintln!("UnknownFrame: code={code} len={}", payload.len());
        }
        _ => {}
    }
})?;

// Block the main thread until you want to stop
std::thread::park();
```
```python [Python]
# Push-callback delivery. The dispatcher thread invokes
# `on_event(event)` for every typed FPSS event under the GIL. Every
# data event carries a typed `event.contract` so user code reads
# `event.contract.symbol` directly — no contract_id side table
# required.
#
# Each event is a typed pyclass (Quote / Trade / Ohlcvc /
# OpenInterest / LoginSuccess / Disconnected / Reconnecting / ...);
# `event.kind` is a snake_case discriminator string per pyclass.
def on_event(event):
    if event.kind == "login_success":
        print(f"Logged in: permissions={event.permissions}")
        return

    # Data events -- all carry received_at_ns and a typed `contract`.
    if event.kind == "quote":
        print(f"Quote: {event.contract.symbol} bid={event.bid} ask={event.ask} "
              f"rx={event.received_at_ns}ns")

    elif event.kind == "trade":
        print(f"Trade: {event.contract.symbol} price={event.price} size={event.size} "
              f"seq={event.sequence} rx={event.received_at_ns}ns")

    elif event.kind == "open_interest":
        print(f"OI: {event.contract.symbol} oi={event.open_interest}")

    elif event.kind == "ohlcvc":
        print(f"OHLCVC: {event.contract.symbol} "
              f"O={event.open} H={event.high} L={event.low} C={event.close} "
              f"vol={event.volume} n={event.count}")

    elif event.kind == "disconnected":
        # `reason` is the RemoveReason discriminant cast to i32.
        print(f"Disconnected: reason={event.reason}")

# `with client.streaming(on_event):` registers the callback on enter
# and pairs `stop_streaming()` + `await_drain(5_000)` on exit.
with client.streaming(on_event):
    client.subscribe(Contract.stock("AAPL").quote())
    import time
    time.sleep(60)
```
```cpp [C++]
client.set_callback([](const tdx::FpssEvent& event) {
    switch (event.kind) {
    case TDX_FPSS_QUOTE: {
        auto& q = event.quote;
        // All price fields are f64 (double) -- direct access, no decoding
        // needed. `q.contract.symbol` carries the resolved symbol.
        std::cout << "Quote: " << q.contract.symbol
                  << " bid=" << q.bid
                  << " ask=" << q.ask
                  << " rx=" << q.received_at_ns << "ns" << std::endl;
        break;
    }
    case TDX_FPSS_TRADE: {
        auto& t = event.trade;
        std::cout << "Trade: " << t.contract.symbol
                  << " price=" << t.price
                  << " size=" << t.size
                  << " seq=" << t.sequence << std::endl;
        break;
    }
    case TDX_FPSS_OPEN_INTEREST: {
        auto& oi = event.open_interest;
        std::cout << "OI: " << oi.contract.symbol
                  << " oi=" << oi.open_interest << std::endl;
        break;
    }
    case TDX_FPSS_OHLCVC: {
        auto& o = event.ohlcvc;
        std::cout << "OHLCVC: " << o.contract.symbol
                  << " O=" << o.open
                  << " H=" << o.high
                  << " L=" << o.low
                  << " C=" << o.close
                  << " vol=" << o.volume << " count=" << o.count << std::endl;
        break;
    }
    case TDX_FPSS_LOGIN_SUCCESS: {
        // Typed control variants — one C struct per FpssControl::*
        // Rust variant. Dispatch on event.kind, read the matching
        // event.<variant> payload.
        if (event.login_success.permissions) {
            std::cout << "LoginSuccess: " << event.login_success.permissions << std::endl;
        }
        break;
    }
    case TDX_FPSS_CONTRACT_ASSIGNED: {
        auto& ca = event.contract_assigned;
        std::cout << "ContractAssigned: id=" << ca.id;
        if (ca.contract.symbol) std::cout << " symbol=" << ca.contract.symbol;
        std::cout << std::endl;
        break;
    }
    case TDX_FPSS_DISCONNECTED:
        std::cout << "Disconnected: reason=" << event.disconnected.reason << std::endl;
        break;
    case TDX_FPSS_RECONNECTING: {
        auto& r = event.reconnecting;
        std::cout << "Reconnecting: reason=" << r.reason
                  << " attempt=" << r.attempt
                  << " delay_ms=" << r.delay_ms << std::endl;
        break;
    }
    case TDX_FPSS_SERVER_ERROR:
        if (event.server_error.message) {
            std::cout << "ServerError: " << event.server_error.message << std::endl;
        }
        break;
    case TDX_FPSS_ERROR:
        if (event.error.message) {
            std::cout << "Error: " << event.error.message << std::endl;
        }
        break;
    case TDX_FPSS_MARKET_OPEN:
        std::cout << "MarketOpen" << std::endl;
        break;
    case TDX_FPSS_MARKET_CLOSE:
        std::cout << "MarketClose" << std::endl;
        break;
    // Other typed control variants (Connected / Reconnected /
    // ReconnectedServer / Restart / Ping / UnknownFrame /
    // UnknownControl / ReqResponse) follow the same pattern —
    // dispatch on event.kind, read event.<variant>.
    case TDX_FPSS_UNKNOWN_FRAME: {
        auto& r = event.unknown_frame;
        std::cout << "UnknownFrame: code=" << (int)r.code
                  << " len=" << r.payload_len << std::endl;
        break;
    }
    default:
        break;
    }
});
```
:::

## Data Event Reference

Every data event carries `received_at_ns` (wall-clock nanoseconds since UNIX epoch, captured at frame decode time).

### Quote (11 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `Arc<Contract>` | Resolved typed contract. Fields: `symbol`, `sec_type: SecType`, `expiration: Option<i32>` (YYYYMMDD), `strike: Option<i32>` (wire integer, thousandths of a dollar), `is_call: Option<bool>` (low-level wire flag). Accessors: `right() -> Option<Right>` (`Right::Call` / `Right::Put`), `strike_dollars() -> Option<f64>` (strike in dollars). |
| `ms_of_day` | `i32` | Milliseconds since midnight ET (exchange timestamp) |
| `bid_size` | `i32` | Bid size in lots |
| `bid_exchange` | `i32` | Bid exchange code |
| `bid` | `f64` | Bid price |
| `bid_condition` | `i32` | Bid condition code |
| `ask_size` | `i32` | Ask size in lots |
| `ask_exchange` | `i32` | Ask exchange code |
| `ask` | `f64` | Ask price |
| `ask_condition` | `i32` | Ask condition code |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

### Trade (16 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `Arc<Contract>` | Resolved typed contract. Fields: `symbol`, `sec_type: SecType`, `expiration: Option<i32>` (YYYYMMDD), `strike: Option<i32>` (wire integer, thousandths of a dollar), `is_call: Option<bool>` (low-level wire flag). Accessors: `right() -> Option<Right>` (`Right::Call` / `Right::Put`), `strike_dollars() -> Option<f64>` (strike in dollars). |
| `ms_of_day` | `i32` | Milliseconds since midnight ET (exchange timestamp) |
| `sequence` | `i32` | Trade sequence number |
| `ext_condition1` | `i32` | Extended condition code 1 |
| `ext_condition2` | `i32` | Extended condition code 2 |
| `ext_condition3` | `i32` | Extended condition code 3 |
| `ext_condition4` | `i32` | Extended condition code 4 |
| `condition` | `i32` | Primary trade condition |
| `size` | `i32` | Trade size in shares/contracts |
| `exchange` | `i32` | Exchange code |
| `price` | `f64` | Trade price |
| `condition_flags` | `i32` | Condition flag bits |
| `price_flags` | `i32` | Price flag bits |
| `volume_type` | `i32` | Volume type indicator |
| `records_back` | `i32` | Records back (correction indicator) |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

::: info Dev server 8-field trades
The dev server (port 20200) sends a simplified 8-field trade format: `ms_of_day`, `condition`, `size`, `exchange`, `price`, `records_back`, `date`. The SDK handles this transparently -- missing fields (`sequence`, `ext_condition*`, `condition_flags`, `price_flags`, `volume_type`) are set to 0.
:::

### OpenInterest (3 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `Arc<Contract>` | Resolved typed contract. Fields: `symbol`, `sec_type: SecType`, `expiration: Option<i32>` (YYYYMMDD), `strike: Option<i32>` (wire integer, thousandths of a dollar), `is_call: Option<bool>` (low-level wire flag). Accessors: `right() -> Option<Right>` (`Right::Call` / `Right::Put`), `strike_dollars() -> Option<f64>` (strike in dollars). |
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `open_interest` | `i32` | Current open interest |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

### Ohlcvc (volume and count are i64)

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `Arc<Contract>` | Resolved typed contract. Fields: `symbol`, `sec_type: SecType`, `expiration: Option<i32>` (YYYYMMDD), `strike: Option<i32>` (wire integer, thousandths of a dollar), `is_call: Option<bool>` (low-level wire flag). Accessors: `right() -> Option<Right>` (`Right::Call` / `Right::Put`), `strike_dollars() -> Option<f64>` (strike in dollars). |
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `open` | `f64` | Open price |
| `high` | `f64` | High price |
| `low` | `f64` | Low price |
| `close` | `f64` | Close price |
| `volume` | **`i64`** | Cumulative volume (i64 to avoid overflow on high-volume symbols) |
| `count` | **`i64`** | Trade count (i64 to avoid overflow) |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

::: tip
OHLCVC bars can come from two sources: wire code 24 (server-sent bars) or trade-derived (computed locally from trade events when OHLCVC derivation is enabled). Set `config.derive_ohlcvc = false` (Rust: `DirectConfig::production().derive_ohlcvc(false)`) to disable local derivation.
:::

## Control Event Reference

Control events are lifecycle and protocol messages. They do not carry `received_at_ns`.

| Event | Fields | Description |
|-------|--------|-------------|
| `LoginSuccess` | `permissions: String` | Authentication succeeded. Permissions string describes subscription tier. |
| `ContractAssigned` | `id: i32`, `contract: Contract` | Server assigned an integer ID to a subscribed contract. Build your contract map from these. |
| `ReqResponse` | `req_id: i32`, `result: StreamResponseType` | Response to a subscribe/unsubscribe request. Result is `Subscribed`, `Error`, `MaxStreamsReached`, or `InvalidPerms`. |
| `MarketOpen` | (none) | Market has opened for the trading day. |
| `MarketClose` | (none) | Market has closed for the trading day. |
| `ServerError` | `message: String` | Non-fatal server error message. |
| `Disconnected` | `reason: RemoveReason` | Connection was terminated by server. Check reason to decide whether to reconnect. |
| `Error` | `message: String` | Protocol-level parse error (corrupt frame, unexpected format). |

### Control Event Dispatch (C++ FFI)

Each `FpssControl::*` Rust variant is exposed as one typed C struct.
Consumers dispatch on `event.kind` (a `TdxFpssEventKind` enum value)
and read the matching `event.<variant>` payload:

| `event.kind` | Typed payload field | Payload fields |
|---|---|---|
| `TDX_FPSS_LOGIN_SUCCESS` | `event.login_success` | `permissions` |
| `TDX_FPSS_CONTRACT_ASSIGNED` | `event.contract_assigned` | `id`, `contract` |
| `TDX_FPSS_REQ_RESPONSE` | `event.req_response` | `req_id`, `result` |
| `TDX_FPSS_MARKET_OPEN` | `event.market_open` | (none) |
| `TDX_FPSS_MARKET_CLOSE` | `event.market_close` | (none) |
| `TDX_FPSS_SERVER_ERROR` | `event.server_error` | `message` |
| `TDX_FPSS_DISCONNECTED` | `event.disconnected` | `reason` (i32 RemoveReason) |
| `TDX_FPSS_RECONNECTING` | `event.reconnecting` | `reason`, `attempt`, `delay_ms` |
| `TDX_FPSS_RECONNECTED` | `event.reconnected` | (none) |
| `TDX_FPSS_ERROR` | `event.error` | `message` |
| `TDX_FPSS_UNKNOWN_FRAME` | `event.unknown_frame` | `code`, `payload`, `payload_len` |
| `TDX_FPSS_CONNECTED` | `event.connected` | (none) |
| `TDX_FPSS_PING` | `event.ping` | `payload`, `payload_len` |
| `TDX_FPSS_RECONNECTED_SERVER` | `event.reconnected_server` | (none) |
| `TDX_FPSS_RESTART` | `event.restart` | (none) |
| `TDX_FPSS_UNKNOWN_CONTROL` | `event.unknown_control` | (none) |

Numeric values of `TdxFpssEventKind` renumber alphabetically; reach
for the symbolic names — they are stable across the rename. Borrowed
pointers (`permissions`, `message`, `payload`, `Contract.symbol`) are
valid only for the duration of the user callback.

## UnknownFrame (unrecognised wire frame)

A frame whose wire code is not yet recognised is delivered as the `FpssControl::UnknownFrame` typed control variant:

| Field | Type | Description |
|-------|------|-------------|
| `code` | `u8` | The raw frame type code |
| `payload` | `Vec<u8>` / `uint8_t*` | The undecoded frame payload |

In C++, `event->unknown_frame.code` and `event->unknown_frame.payload` with `event->unknown_frame.payload_len`. In Python, `event.kind == "unknown_frame"` with `event.code` and `event.payload`.

## SDK-Specific Event Representations

### FFI (C++)

Events are `#[repr(C)]` tagged structs, **not JSON**. The top-level
`TdxFpssEvent` struct has a `kind` tag (`TdxFpssEventKind` enum) and
one embedded `#[repr(C)]` payload per data variant + per typed
control variant + the raw-bytes fallback. Check `kind` first, then
access the corresponding field. Only the field matching `kind` is
valid; sibling fields are zero-filled.

The full enum + struct layout lives in
`sdks/cpp/include/fpss_event_structs.h.inc` (generated from
`fpss_event_schema.toml`).

### C++

The unified `tdx::UnifiedClient` accepts a push callback via `client.set_callback([](const tdx::FpssEvent& event) { ... })`. The dispatcher thread invokes the lambda for every typed event under `catch_unwind`. The dedicated `tdx::FpssClient` exposes the same `set_callback(fn)` shape plus `reconnect()` for handler-rebinding reconnects. The typed event payload is the same on both:

- `event.kind` -- `TdxFpssEventKind` enum
- `event.quote` / `event.trade` / etc. -- direct struct member access
- `event.quote.contract.symbol` (and `expiration`, `right`, `strike` on options, gated by `has_expiration` / `has_right` / `has_strike`; `right` is the ASCII byte `'C'` / `'P'`) — typed contract resolved before the SDK hands the event to user code
- All price fields are `double` (f64) -- access them directly

### Python

The unified `ThetaDataDxClient` dispatches every event through `start_streaming(callback)` (or the `streaming(callback)` context manager). The callable runs on the dispatcher thread under the GIL, wrapped in `catch_unwind`.

The surfaced event is a typed pyclass — one per `FpssData` variant and one per `FpssControl` variant. Branch on `event.kind` and read the variant's typed payload directly:

- **Data variants** — `Quote`, `Trade`, `OpenInterest`, `Ohlcvc`. `event.kind` is `"quote"`, `"trade"`, `"open_interest"`, `"ohlcvc"`. Each carries a typed `event.contract` with `symbol: str`, `sec_type: str` (`"STOCK"` / `"OPTION"` / `"INDEX"` / `"RATE"`), `expiration: Optional[int]` (YYYYMMDD), `right: Optional[str]` (`"C"` / `"P"`, `None` for non-options), `strike_dollars: Optional[float]` (strike in dollars), and `strike: Optional[int]` (wire integer, thousandths of a dollar). Price fields (`bid`, `ask`, `price`, `open`, `high`, `low`, `close`) are pre-decoded to `float`. All data variants include `received_at_ns: int`.
- **Control variants** — `LoginSuccess`, `ContractAssigned`, `ReqResponse`, `MarketOpen`, `MarketClose`, `ServerError`, `Disconnected`, `Reconnecting`, `Reconnected`, `Error`, `UnknownFrame`, `UnknownControl`, `Connected`, `Ping`, `ReconnectedServer`, `Restart`. `event.kind` matches the snake_case form (`"login_success"`, `"contract_assigned"`, `"disconnected"`, etc.). Each variant exposes only the fields its Rust counterpart carries — e.g. `LoginSuccess.permissions: str`, `Disconnected.{reason: int, reason_name: str}` (`reason_name` is the `RemoveReason` enum name like `"TooManyRequests"`), `Reconnecting.{reason: int, reason_name: str, attempt: int, delay_ms: int}`, `ContractAssigned.{id: int, contract: Contract}`, `ServerError.message: str`, `UnknownFrame.{code: int, payload: bytes}`. Variants with no payload (`MarketOpen`, `MarketClose`, `Reconnected`, `Connected`, `Restart`, `ReconnectedServer`, `UnknownControl`) carry only `kind`.

## Streaming Methods Reference

### Rust (`ThetaDataDxClient`)

| Method | Description |
|--------|-------------|
| `start_streaming(callback)` | Begin streaming with an event callback (reads `derive_ohlcvc` from config) |
| `subscribe(spec)` | Polymorphic subscribe — `spec` is `Contract::stock("AAPL").quote()`, `Contract::option(...)?.trade()`, `SecType::Stock.full_trades()`, etc. |
| `subscribe_many(specs)` | Bulk subscribe over an iterable of specs. |
| `unsubscribe(spec)` | Polymorphic unsubscribe — same spec shape as `subscribe`. |
| `unsubscribe_many(specs)` | Bulk unsubscribe. |
| `reconnect_streaming(handler)` | Reconnect with new handler, re-subscribe all previous subs |
| `is_streaming()` | Check if streaming is active |
| `await_drain(timeout)` | Block until the previous session's consumer thread has fully drained (returns `true` on quiescence, `false` on timeout) |
| `active_subscriptions()` | Get active per-contract subscriptions |
| `stop_streaming()` | Stop the streaming connection |

### Python (`ThetaDataDxClient`)

| Method | Description |
|--------|-------------|
| `start_streaming(callback)` | Register a callable; the dispatcher thread invokes `callback(event)` under the GIL for every typed FPSS event |
| `streaming(callback)` | Context-manager wrapper — `with tdx.streaming(callback) as session:` registers the callback on enter and pairs `stop_streaming()` + `await_drain(5_000)` on exit |
| `subscribe(spec)` | Polymorphic subscribe — `spec` is `Contract.stock("AAPL").quote()`, `Contract.option(...).trade()`, `SecType.Stock.full_trades()`, etc. |
| `subscribe_many(specs)` | Bulk subscribe over an iterable of specs. |
| `unsubscribe(spec)` | Polymorphic unsubscribe — same spec shape as `subscribe`. |
| `unsubscribe_many(specs)` | Bulk unsubscribe. |
| `active_subscriptions()` | Get active subscriptions |
| `reconnect()` | Reconnect streaming and re-subscribe previous subscriptions (callback registered at `start_streaming` is reused) |
| `await_drain(timeout_ms)` | Block until the previous session's consumer has drained |
| `is_streaming()` | Check if streaming is active |
| `dropped_event_count()` | Cumulative count of events the TLS reader could not publish because the consumer fell behind |
| `stop_streaming()` | Graceful shutdown of streaming |

### C++ (`tdx::UnifiedClient`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `connect` (static) | `(creds, config) -> UnifiedClient` | Construct the unified handle |
| `set_callback` | `(std::function<void(const FpssEvent&)>) -> void` | Register the push callback; the dispatcher thread invokes it under `catch_unwind` |
| `subscribe` | `(FluentSubscription) -> void` | Polymorphic subscribe — `tdx::Contract::stock("AAPL").quote()`, `tdx::Contract::option(...).trade()`, `tdx::SecType::Stock.full_trades()`, etc. |
| `subscribe_many` | `(initializer_list<FluentSubscription>) -> void` | Bulk subscribe; throws on first error |
| `unsubscribe` | `(FluentSubscription) -> void` | Polymorphic unsubscribe — same spec shape as `subscribe` |
| `unsubscribe_many` | `(initializer_list<FluentSubscription>) -> void` | Bulk unsubscribe |
| `is_streaming` | `() -> bool` | Whether the streaming session is live |
| `reconnect` | `() -> void` | Reconnect streaming and re-apply every previously active subscription |
| `stop_streaming` | `() -> void` | Stop streaming; historical access remains available |
| `flat_files` | `() -> FlatFiles` | Borrow the FLATFILES surface (lifetime bounded by `*this`) |
| `get` | `() -> const TdxUnified*` | Raw handle for direct C-ABI calls (`tdx_unified_set_callback`, `tdx_fpss_await_drain`, `tdx_unified_stop_streaming`) |

For handler-rebinding reconnects on the standalone path, use the dedicated `tdx::FpssClient` (`set_callback`, `reconnect`, `shutdown`, `dropped_events`, `is_authenticated`, `active_subscriptions`).

All price fields are `double` (f64) -- access them directly. Data events carry a typed `contract` (with `symbol`, `sec_type`, etc.); read `event.quote.contract.symbol` directly instead of looking up the integer ID.
