---
title: Handling Events
description: Process data and control events from the FPSS streaming connection - quotes, trades, open interest, OHLCVC, control messages, and raw data.
---

# Handling Events

## Receive Events

::: code-group
```rust [Rust]
tdx.start_streaming(|event: &FpssEvent| {
    match event {
        // --- Data events ---
        FpssEvent::Data(FpssData::Quote {
            contract_id, ms_of_day, bid_f64, ask_f64, bid_size, ask_size,
            received_at_ns, ..
        }) => {
            println!("Quote: id={contract_id} bid={bid_f64:.2} ask={ask_f64:.2} rx={received_at_ns}ns");
        }
        FpssEvent::Data(FpssData::Trade {
            contract_id, price_f64, size, sequence, received_at_ns, ..
        }) => {
            println!("Trade: id={contract_id} price={price_f64:.2} size={size} seq={sequence}");
        }
        FpssEvent::Data(FpssData::OpenInterest {
            contract_id, open_interest, received_at_ns, ..
        }) => {
            println!("OI: id={contract_id} oi={open_interest} rx={received_at_ns}ns");
        }
        FpssEvent::Data(FpssData::Ohlcvc {
            contract_id, open_f64, high_f64, low_f64, close_f64,
            volume, count, received_at_ns, ..
        }) => {
            // volume and count are i64 to avoid overflow on high-volume symbols
            println!("OHLCVC: id={contract_id} O={open_f64:.2} H={high_f64:.2} L={low_f64:.2} C={close_f64:.2} vol={volume} n={count}");
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

        // --- Raw undecoded fallback ---
        FpssEvent::RawData { code, payload } => {
            eprintln!("Raw frame: code={code} len={}", payload.len());
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
        print(f"Logged in: {event.get('detail')}")
        continue

    # Data events -- all carry received_at_ns
    if event["kind"] == "quote":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Quote: {symbol} bid={event['bid']} ask={event['ask']} "
              f"rx={event['received_at_ns']}ns")

    elif event["kind"] == "trade":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Trade: {symbol} price={event['price']} size={event['size']} "
              f"seq={event['sequence']} rx={event['received_at_ns']}ns")

    elif event["kind"] == "open_interest":
        print(f"OI: contract={event['contract_id']} oi={event['open_interest']}")

    elif event["kind"] == "ohlcvc":
        print(f"OHLCVC: contract={event['contract_id']} "
              f"O={event['open']} H={event['high']} L={event['low']} C={event['close']} "
              f"vol={event['volume']} n={event['count']}")

    elif event["kind"] == "disconnected":
        print(f"Disconnected: {event.get('detail')}")
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

    switch event.Kind {
    case thetadatadx.FpssQuoteEvent:
        q := event.Quote
        // Bid and Ask are pre-decoded to float64
        fmt.Printf("Quote: contract=%d bid=%.4f ask=%.4f rx=%dns\n",
            q.ContractID, q.Bid, q.Ask, q.ReceivedAtNs)

    case thetadatadx.FpssTradeEvent:
        t := event.Trade
        // Price is pre-decoded to float64
        fmt.Printf("Trade: contract=%d price=%.4f size=%d seq=%d\n",
            t.ContractID, t.Price, t.Size, t.Sequence)

    case thetadatadx.FpssOpenInterestEvent:
        oi := event.OpenInterest
        fmt.Printf("OI: contract=%d oi=%d\n", oi.ContractID, oi.OpenInterest)

    case thetadatadx.FpssOhlcvcEvent:
        o := event.Ohlcvc
        // OHLC prices are pre-decoded to float64
        fmt.Printf("OHLCVC: contract=%d O=%.4f H=%.4f L=%.4f C=%.4f vol=%d count=%d\n",
            o.ContractID, o.Open, o.High, o.Low, o.Close, o.Volume, o.Count)

    case thetadatadx.FpssControlEvent:
        ctrl := event.Control
        fmt.Printf("Control: kind=%d detail=%s\n", ctrl.Kind, ctrl.Detail)
    }
}
```
```cpp [C++]
while (true) {
    tdx::FpssEventPtr event = fpss.next_event(5000); // 5s timeout
    if (!event) {
        continue; // timeout
    }

    switch (event->kind) {
    case TDX_FPSS_QUOTE: {
        auto& q = event->quote;
        // Use tdx::price_to_f64() to decode streaming prices
        std::cout << "Quote: contract=" << q.contract_id
                  << " bid=" << tdx::price_to_f64(q.bid, q.price_type)
                  << " ask=" << tdx::price_to_f64(q.ask, q.price_type)
                  << " rx=" << q.received_at_ns << "ns" << std::endl;
        break;
    }
    case TDX_FPSS_TRADE: {
        auto& t = event->trade;
        std::cout << "Trade: contract=" << t.contract_id
                  << " price=" << tdx::price_to_f64(t.price, t.price_type)
                  << " size=" << t.size
                  << " seq=" << t.sequence << std::endl;
        break;
    }
    case TDX_FPSS_OPEN_INTEREST: {
        auto& oi = event->open_interest;
        std::cout << "OI: contract=" << oi.contract_id
                  << " oi=" << oi.open_interest << std::endl;
        break;
    }
    case TDX_FPSS_OHLCVC: {
        auto& o = event->ohlcvc;
        std::cout << "OHLCVC: contract=" << o.contract_id
                  << " O=" << tdx::price_to_f64(o.open, o.price_type)
                  << " H=" << tdx::price_to_f64(o.high, o.price_type)
                  << " L=" << tdx::price_to_f64(o.low, o.price_type)
                  << " C=" << tdx::price_to_f64(o.close, o.price_type)
                  << " vol=" << o.volume << " count=" << o.count << std::endl;
        break;
    }
    case TDX_FPSS_CONTROL: {
        auto& c = event->control;
        std::cout << "Control: kind=" << c.kind;
        if (c.detail) std::cout << " detail=" << c.detail;
        std::cout << std::endl;
        break;
    }
    case TDX_FPSS_RAW_DATA: {
        auto& r = event->raw_data;
        std::cout << "Raw: code=" << (int)r.code
                  << " len=" << r.payload_len << std::endl;
        break;
    }
    }
}
```
:::

## Data Event Reference

Every data event carries `received_at_ns` (wall-clock nanoseconds since UNIX epoch, captured at frame decode time).

### Quote (11 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract_id` | `i32` | Server-assigned contract identifier |
| `ms_of_day` | `i32` | Milliseconds since midnight ET (exchange timestamp) |
| `bid_size` | `i32` | Bid size in lots |
| `bid_exchange` | `i32` | Bid exchange code |
| `bid` | `i32` | Bid price (raw integer, decode with `price_type`) |
| `bid_f64` | `f64` | Pre-decoded bid price (use this in most cases) |
| `bid_condition` | `i32` | Bid condition code |
| `ask_size` | `i32` | Ask size in lots |
| `ask_exchange` | `i32` | Ask exchange code |
| `ask` | `i32` | Ask price (raw integer, decode with `price_type`) |
| `ask_f64` | `f64` | Pre-decoded ask price (use this in most cases) |
| `ask_condition` | `i32` | Ask condition code |
| `price_type` | `i32` | Price encoding exponent |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

### Trade (16 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract_id` | `i32` | Server-assigned contract identifier |
| `ms_of_day` | `i32` | Milliseconds since midnight ET (exchange timestamp) |
| `sequence` | `i32` | Trade sequence number |
| `ext_condition1` | `i32` | Extended condition code 1 |
| `ext_condition2` | `i32` | Extended condition code 2 |
| `ext_condition3` | `i32` | Extended condition code 3 |
| `ext_condition4` | `i32` | Extended condition code 4 |
| `condition` | `i32` | Primary trade condition |
| `size` | `i32` | Trade size in shares/contracts |
| `exchange` | `i32` | Exchange code |
| `price` | `i32` | Trade price (raw integer, decode with `price_type`) |
| `price_f64` | `f64` | Pre-decoded trade price (use this in most cases) |
| `condition_flags` | `i32` | Condition flag bits |
| `price_flags` | `i32` | Price flag bits |
| `volume_type` | `i32` | Volume type indicator |
| `records_back` | `i32` | Records back (correction indicator) |
| `price_type` | `i32` | Price encoding exponent |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

::: info Dev server 8-field trades
The dev server (port 20200) sends a simplified 8-field trade format: `ms_of_day`, `condition`, `size`, `exchange`, `price`, `records_back`, `price_type`, `date`. The SDK handles this transparently -- missing fields (`sequence`, `ext_condition*`, `condition_flags`, `price_flags`, `volume_type`) are set to 0.
:::

### OpenInterest (3 fields + received_at_ns)

| Field | Type | Description |
|-------|------|-------------|
| `contract_id` | `i32` | Server-assigned contract identifier |
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `open_interest` | `i32` | Current open interest |
| `date` | `i32` | Date as YYYYMMDD integer |
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch |

### Ohlcvc (volume and count are i64)

| Field | Type | Description |
|-------|------|-------------|
| `contract_id` | `i32` | Server-assigned contract identifier |
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `open` | `i32` | Open price (raw integer) |
| `open_f64` | `f64` | Pre-decoded open price |
| `high` | `i32` | High price (raw integer) |
| `high_f64` | `f64` | Pre-decoded high price |
| `low` | `i32` | Low price (raw integer) |
| `low_f64` | `f64` | Pre-decoded low price |
| `close` | `i32` | Close price (raw integer) |
| `close_f64` | `f64` | Pre-decoded close price |
| `volume` | **`i64`** | Cumulative volume (i64 to avoid overflow on high-volume symbols) |
| `count` | **`i64`** | Trade count (i64 to avoid overflow) |
| `price_type` | `i32` | Price encoding exponent |
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

### Control Event Kind Codes (Go/C++ FFI)

In the Go and C++ SDKs, control events carry an integer `kind` field:

| Kind | Event |
|------|-------|
| 0 | `LoginSuccess` |
| 1 | `ContractAssigned` |
| 2 | `ReqResponse` |
| 3 | `MarketOpen` |
| 4 | `MarketClose` |
| 5 | `ServerError` |
| 6 | `Disconnected` |
| 7 | `Error` |

## RawData (undecoded fallback)

If a frame cannot be decoded (too short, corrupt, or unknown code), it is delivered as a `RawData` event:

| Field | Type | Description |
|-------|------|-------------|
| `code` | `u8` | The raw frame type code |
| `payload` | `Vec<u8>` / `[]byte` / `uint8_t*` | The undecoded frame payload |

In Go, these are `event.RawCode` and `event.RawPayload`. In C++, `event->raw_data.code` and `event->raw_data.payload` with `event->raw_data.payload_len`.

## SDK-Specific Event Representations

### FFI (Go / C++)

Events are `#[repr(C)]` tagged structs, **not JSON**. The top-level `TdxFpssEvent` struct has a `kind` tag (`TdxFpssEventKind` enum) and union-style fields:

```c
typedef struct {
    TdxFpssEventKind kind;   // 0=Quote, 1=Trade, 2=OI, 3=Ohlcvc, 4=Control, 5=RawData
    TdxFpssQuote quote;
    TdxFpssTrade trade;
    TdxFpssOpenInterest open_interest;
    TdxFpssOhlcvc ohlcvc;
    TdxFpssControl control;
    TdxFpssRawData raw_data;
} TdxFpssEvent;
```

Check `kind` first, then access the corresponding field. Only the field matching `kind` is valid.

### Go

`NextEvent(timeoutMs)` returns `*FpssEvent` with typed Go struct fields:

- `event.Kind` -- `FpssEventKind` (int constant)
- `event.Quote` -- `*FpssQuote` (non-nil when Kind is `FpssQuoteEvent`)
- `event.Trade` -- `*FpssTrade` (non-nil when Kind is `FpssTradeEvent`)
- `event.OpenInterest` -- `*FpssOpenInterestData` (non-nil when Kind is `FpssOpenInterestEvent`)
- `event.Ohlcvc` -- `*FpssOhlcvc` (non-nil when Kind is `FpssOhlcvcEvent`)
- `event.Control` -- `*FpssControlData` (non-nil when Kind is `FpssControlEvent`)

Price fields (`Bid`, `Ask`, `Price`, `Open`, `High`, `Low`, `Close`) are pre-decoded to `float64`. Raw integer values are available as `BidRaw`, `AskRaw`, `PriceRaw`, `OpenRaw`, etc. The `PriceToF64(value, priceType)` helper remains exported for custom decoding.

### C++

`next_event(timeout_ms)` returns `FpssEventPtr` which is `std::unique_ptr<TdxFpssEvent, FpssEventDeleter>` (RAII, automatically freed on scope exit):

- `event->kind` -- `TdxFpssEventKind` enum
- `event->quote` / `event->trade` / etc. -- direct struct member access
- Use `tdx::price_to_f64(value, price_type)` for price decoding

### Python

`next_event(timeout_ms)` returns a Python `dict` with all fields as key-value pairs:

- `event["kind"]` -- string: `"quote"`, `"trade"`, `"open_interest"`, `"ohlcvc"`, `"login_success"`, `"contract_assigned"`, `"req_response"`, `"market_open"`, `"market_close"`, `"server_error"`, `"disconnected"`, `"error"`
- Price fields in quotes and trades are pre-decoded to `float` (bid, ask, price, open, high, low, close)
- Trade events also include `price_raw` (original integer) and `price_type`
- All data events include `received_at_ns` as an integer

## Streaming Methods Reference

### Rust (`ThetaDataDx`)

| Method | Description |
|--------|-------------|
| `start_streaming(callback)` | Begin streaming with an event callback (reads `derive_ohlcvc` from config) |
| `subscribe_quotes(contract)` | Subscribe to quote data |
| `subscribe_trades(contract)` | Subscribe to trade data |
| `subscribe_open_interest(contract)` | Subscribe to open interest |
| `subscribe_full_trades(sec_type)` | Subscribe to all trades for a security type (firehose) |
| `subscribe_full_open_interest(sec_type)` | Subscribe to all OI for a security type (firehose) |
| `unsubscribe_quotes(contract)` | Unsubscribe from quotes |
| `unsubscribe_trades(contract)` | Unsubscribe from trades |
| `unsubscribe_open_interest(contract)` | Unsubscribe from OI |
| `unsubscribe_full_trades(sec_type)` | Unsubscribe from all trades for a security type |
| `unsubscribe_full_open_interest(sec_type)` | Unsubscribe from all OI for a security type |
| `reconnect_streaming(handler)` | Reconnect with new handler, re-subscribe all previous subs |
| `is_streaming()` | Check if FPSS is active |
| `contract_lookup(id)` | Look up contract by server-assigned ID |
| `contract_map()` | Get current contract ID mapping |
| `active_subscriptions()` | Get active per-contract subscriptions |
| `active_full_subscriptions()` | Get active firehose subscriptions |
| `stop_streaming()` | Stop the streaming connection |

### Python (`ThetaDataDx`)

| Method | Description |
|--------|-------------|
| `start_streaming()` | Connect to FPSS streaming servers (reads `derive_ohlcvc` from config) |
| `subscribe_quotes(symbol)` | Subscribe to quote data |
| `subscribe_trades(symbol)` | Subscribe to trade data |
| `subscribe_open_interest(symbol)` | Subscribe to open interest |
| `subscribe_full_trades(sec_type)` | Subscribe to all trades for a security type |
| `subscribe_full_open_interest(sec_type)` | Subscribe to all OI for a security type |
| `unsubscribe_quotes(symbol)` | Unsubscribe from quotes |
| `unsubscribe_trades(symbol)` | Unsubscribe from trades |
| `unsubscribe_open_interest(symbol)` | Unsubscribe from OI |
| `unsubscribe_full_trades(sec_type)` | Unsubscribe from all trades |
| `unsubscribe_full_open_interest(sec_type)` | Unsubscribe from all OI |
| `next_event(timeout_ms=5000)` | Poll next event (returns dict or `None`) |
| `stop_streaming()` | Graceful shutdown of streaming |

### Go (`FpssClient`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `SubscribeQuotes` | `(symbol string) (int, error)` | Subscribe to quotes |
| `SubscribeTrades` | `(symbol string) (int, error)` | Subscribe to trades |
| `SubscribeOpenInterest` | `(symbol string) (int, error)` | Subscribe to OI |
| `SubscribeFullTrades` | `(secType string) (int, error)` | Subscribe to all trades for a security type |
| `SubscribeFullOpenInterest` | `(secType string) (int, error)` | Subscribe to all OI for a security type |
| `UnsubscribeQuotes` | `(symbol string) (int, error)` | Unsubscribe from quotes |
| `UnsubscribeTrades` | `(symbol string) (int, error)` | Unsubscribe from trades |
| `UnsubscribeOpenInterest` | `(symbol string) (int, error)` | Unsubscribe from OI |
| `UnsubscribeFullTrades` | `(secType string) (int, error)` | Unsubscribe from all trades |
| `UnsubscribeFullOpenInterest` | `(secType string) (int, error)` | Unsubscribe from all OI |
| `NextEvent` | `(timeoutMs uint64) (*FpssEvent, error)` | Poll next event as typed struct (nil on timeout) |
| `IsAuthenticated` | `() bool` | Check FPSS auth status |
| `ContractLookup` | `(id int) (string, error)` | Look up contract by server-assigned ID |
| `ActiveSubscriptions` | `() ([]Subscription, error)` | Get active subscriptions as typed structs |
| `Shutdown` | `()` | Graceful shutdown |
| `Close` | `()` | Free the FPSS handle (call after Shutdown) |

Helper: `PriceToF64(value int32, priceType int32) float64` -- decode raw integer prices. Note: FPSS event price fields are pre-decoded to `float64` as of v5.2; this helper is for custom use cases.

### C++ (`tdx::FpssClient`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `subscribe_quotes` | `(symbol) -> int` | Subscribe to quotes |
| `subscribe_trades` | `(symbol) -> int` | Subscribe to trades |
| `subscribe_open_interest` | `(symbol) -> int` | Subscribe to OI |
| `subscribe_full_trades` | `(sec_type) -> int` | Subscribe to all trades for a security type |
| `subscribe_full_open_interest` | `(sec_type) -> int` | Subscribe to all OI for a security type |
| `unsubscribe_quotes` | `(symbol) -> int` | Unsubscribe from quotes |
| `unsubscribe_trades` | `(symbol) -> int` | Unsubscribe from trades |
| `unsubscribe_open_interest` | `(symbol) -> int` | Unsubscribe from OI |
| `unsubscribe_full_trades` | `(sec_type) -> int` | Unsubscribe from all trades |
| `unsubscribe_full_open_interest` | `(sec_type) -> int` | Unsubscribe from all OI |
| `next_event` | `(timeout_ms) -> FpssEventPtr` | Poll next event (nullptr on timeout). RAII: auto-freed on scope exit. |
| `is_authenticated` | `() -> bool` | Check FPSS auth status |
| `contract_lookup` | `(id) -> std::optional<std::string>` | Look up contract by server-assigned ID |
| `active_subscriptions` | `() -> std::string` | Get active subscriptions |
| `shutdown` | `() -> void` | Graceful shutdown |

Helper: `tdx::price_to_f64(int32_t value, int32_t price_type) -> double` -- decode raw integer prices. For historical tick types, convenience functions are also available: `tdx::trade_price_f64(tick)`, `tdx::bid_f64(q)`, `tdx::open_f64(bar)`, etc.
