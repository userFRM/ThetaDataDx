---
title: Real-Time Streaming
description: Subscribe to live market data via ThetaData's FPSS servers with quote, trade, open interest, and OHLCVC streaming across Rust, Python, Go, and C++.
---

# Real-Time Streaming

Real-time market data is delivered via ThetaData's FPSS (Feed Processing Streaming Server) over persistent TLS/TCP connections. FPSS delivers live quotes, trades, open interest, and OHLCVC bars as typed, zero-copy events.

Each SDK exposes FPSS differently:

- **Rust** -- Fully synchronous callback model. Events dispatched through an LMAX Disruptor ring buffer. No Tokio on the streaming hot path.
- **Python** -- Polling model with `next_event()`. Events returned as Python dicts with all fields.
- **Go** -- Polling model with `NextEvent()`. Events returned as typed `*FpssEvent` structs. Price fields are pre-decoded to `float64`; raw integers available as `*Raw` fields.
- **C++** -- Polling model with `next_event()`. Events returned as `FpssEventPtr` (`unique_ptr<TdxFpssEvent>`, RAII). `#[repr(C)]` layout-compatible structs.

::: warning No JSON in FFI
Go and C++ receive typed `#[repr(C)]` structs directly from Rust -- not JSON. All field access is zero-copy struct member access.
:::

## Server Environments

| Config | FPSS Ports | Purpose |
|--------|-----------|---------|
| `DirectConfig::production()` | 20000, 20001 | Live production data |
| `DirectConfig::dev()` | 20200, 20201 | Historical day replay at max speed (testing when markets are closed) |
| `DirectConfig::stage()` | 20100, 20101 | Staging/testing (frequent reboots, unstable) |

## Connect

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
// Production, dev, or stage:
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
// let tdx = ThetaDataDx::connect(&creds, DirectConfig::dev()).await?;
// let tdx = ThetaDataDx::connect(&creds, DirectConfig::stage()).await?;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, received_at_ns, .. }) => {
            println!("Quote: contract={contract_id} bid={bid} ask={ask} rx={received_at_ns}ns");
        }
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, received_at_ns, .. }) => {
            println!("Trade: contract={contract_id} price={price} size={size} rx={received_at_ns}ns");
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
# Production, dev, or stage:
tdx = ThetaDataDx(creds, Config.production())
# tdx = ThetaDataDx(creds, Config.dev())
# tdx = ThetaDataDx(creds, Config.stage())

tdx.start_streaming()
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

// Production, dev, or stage:
config := thetadatadx.ProductionConfig()
// config := thetadatadx.DevConfig()
// config := thetadatadx.StageConfig()
defer config.Close()

fpss, _ := thetadatadx.NewFpssClient(creds, config)
defer fpss.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
// Production, dev, or stage:
auto config = tdx::Config::production();
// auto config = tdx::Config::dev();
// auto config = tdx::Config::stage();
tdx::FpssClient fpss(creds, config);
```
:::

## Flush Mode

`FpssFlushMode` controls the latency/syscall tradeoff:

| Mode | Flush trigger | Added latency | Best for |
|------|--------------|---------------|----------|
| `Batched` (default) | PING frames every ~100ms | Up to 100ms | Production throughput |
| `Immediate` | Every frame write | None | Lowest latency |

```rust
use thetadatadx::config::FpssFlushMode;
let mut config = DirectConfig::production();
config.fpss_flush_mode = FpssFlushMode::Immediate;
```

## Subscribe

::: code-group
```rust [Rust]
// Stock quotes
let req_id = tdx.subscribe_quotes(&Contract::stock("AAPL"))?;

// Stock trades
tdx.subscribe_trades(&Contract::stock("MSFT"))?;

// Option quotes
let opt = Contract::option("SPY", 20261218, true, 60000);
tdx.subscribe_quotes(&opt)?;

// Open interest
tdx.subscribe_open_interest(&Contract::stock("AAPL"))?;

// Firehose: all trades for a security type
tdx.subscribe_full_trades(SecType::Stock)?;
tdx.subscribe_full_open_interest(SecType::Option)?;
```
```python [Python]
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")
tdx.subscribe_open_interest("SPY")
tdx.subscribe_full_trades("STOCK")
tdx.subscribe_full_open_interest("OPTION")
```
```go [Go]
fpss.SubscribeQuotes("AAPL")
fpss.SubscribeTrades("MSFT")
fpss.SubscribeOpenInterest("AAPL")
fpss.SubscribeFullTrades("STOCK")
fpss.SubscribeFullOpenInterest("OPTION")
```
```cpp [C++]
fpss.subscribe_quotes("AAPL");
fpss.subscribe_trades("MSFT");
fpss.subscribe_open_interest("AAPL");
fpss.subscribe_full_trades("STOCK");
fpss.subscribe_full_open_interest("OPTION");
```
:::

## Receive Events

::: code-group
```rust [Rust]
tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote {
            contract_id, ms_of_day, bid, ask, bid_size, ask_size,
            price_type, received_at_ns, ..
        }) => {
            let bid_price = Price::new(*bid, *price_type);
            let ask_price = Price::new(*ask, *price_type);
            println!("Quote: id={contract_id} bid={bid_price} ask={ask_price}");
        }
        FpssEvent::Data(FpssData::Trade {
            contract_id, price, size, price_type, sequence, received_at_ns, ..
        }) => {
            let trade_price = Price::new(*price, *price_type);
            println!("Trade: id={contract_id} price={trade_price} size={size}");
        }
        FpssEvent::Data(FpssData::OpenInterest {
            contract_id, open_interest, received_at_ns, ..
        }) => {
            println!("OI: id={contract_id} oi={open_interest}");
        }
        FpssEvent::Data(FpssData::Ohlcvc {
            contract_id, open, high, low, close, volume, count, received_at_ns, ..
        }) => {
            // volume and count are i64 to avoid overflow
            println!("OHLCVC: id={contract_id} O={open} H={high} L={low} C={close}");
        }
        FpssEvent::Control(ctrl) => {
            println!("Control: {:?}", ctrl);
        }
        FpssEvent::RawData { code, payload } => {
            eprintln!("Raw frame: code={code} len={}", payload.len());
        }
        _ => {}
    }
})?;
```
```python [Python]
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue

    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
        continue

    if event["kind"] == "quote":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Quote: {symbol} bid={event['bid']} ask={event['ask']} "
              f"rx={event['received_at_ns']}ns")

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
        print(f"Disconnected: {event.get('detail')}")
        break
```
```go [Go]
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
        fmt.Printf("Quote: contract=%d bid=%.4f ask=%.4f rx=%dns\n",
            q.ContractID, q.Bid, q.Ask, q.ReceivedAtNs)

    case thetadatadx.FpssTradeEvent:
        t := event.Trade
        // Price is pre-decoded to float64
        fmt.Printf("Trade: contract=%d price=%.4f size=%d\n",
            t.ContractID, t.Price, t.Size)

    case thetadatadx.FpssOpenInterestEvent:
        oi := event.OpenInterest
        fmt.Printf("OI: contract=%d oi=%d\n", oi.ContractID, oi.OpenInterest)

    case thetadatadx.FpssOhlcvcEvent:
        o := event.Ohlcvc
        fmt.Printf("OHLCVC: contract=%d vol=%d count=%d\n",
            o.ContractID, o.Volume, o.Count)

    case thetadatadx.FpssControlEvent:
        ctrl := event.Control
        fmt.Printf("Control: kind=%d detail=%s\n", ctrl.Kind, ctrl.Detail)
    }
}
```
```cpp [C++]
while (true) {
    auto event = fpss.next_event(5000);
    if (!event) continue;

    switch (event->kind) {
    case TDX_FPSS_QUOTE: {
        auto& q = event->quote;
        double bid = tdx::price_to_f64(q.bid, q.price_type);
        double ask = tdx::price_to_f64(q.ask, q.price_type);
        std::cout << "Quote: contract=" << q.contract_id
                  << " bid=" << bid << " ask=" << ask
                  << " rx=" << q.received_at_ns << "ns" << std::endl;
        break;
    }
    case TDX_FPSS_TRADE: {
        auto& t = event->trade;
        double price = tdx::price_to_f64(t.price, t.price_type);
        std::cout << "Trade: contract=" << t.contract_id
                  << " price=" << price << " size=" << t.size << std::endl;
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
                  << " vol=" << o.volume << " count=" << o.count << std::endl;
        break;
    }
    case TDX_FPSS_CONTROL: {
        auto& c = event->control;
        std::cout << "Control: kind=" << c.kind;
        if (c.detail) std::cout << " " << c.detail;
        std::cout << std::endl;
        break;
    }
    default: break;
    }
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
contract, err := fpss.ContractLookup(42)
if err != nil {
    log.Fatal(err)
}
fmt.Println("Contract:", contract)

subs, _ := fpss.ActiveSubscriptions()
for _, s := range subs {
    fmt.Printf("  %s: %s\n", s.Kind, s.Contract)
}
```
```cpp [C++]
auto contract = fpss.contract_lookup(42);
if (contract.has_value()) {
    std::cout << "Contract: " << contract.value() << std::endl;
}

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
tdx.unsubscribe_full_trades(SecType::Stock)?;
tdx.unsubscribe_full_open_interest(SecType::Option)?;
```
```python [Python]
tdx.unsubscribe_quotes("AAPL")
tdx.unsubscribe_trades("MSFT")
tdx.unsubscribe_open_interest("SPY")
tdx.unsubscribe_full_trades("STOCK")
tdx.unsubscribe_full_open_interest("OPTION")
```
```go [Go]
fpss.UnsubscribeQuotes("AAPL")
fpss.UnsubscribeTrades("MSFT")
fpss.UnsubscribeOpenInterest("AAPL")
fpss.UnsubscribeFullTrades("STOCK")
fpss.UnsubscribeFullOpenInterest("OPTION")
```
```cpp [C++]
fpss.unsubscribe_quotes("AAPL");
fpss.unsubscribe_trades("MSFT");
fpss.unsubscribe_open_interest("AAPL");
fpss.unsubscribe_full_trades("STOCK");
fpss.unsubscribe_full_open_interest("OPTION");
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

ThetaDataDx provides `reconnect_streaming()` for automatic reconnection with subscription recovery:

```rust
use tdbe::types::enums::RemoveReason;

match thetadatadx::fpss::reconnect_delay(reason) {
    None => {
        // Permanent error (bad credentials, etc.) -- do NOT retry
        eprintln!("Permanent disconnect: {:?}", reason);
    }
    Some(delay_ms) => {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        // Saves subs, stops, reconnects, and re-subscribes everything
        tdx.reconnect_streaming(handler)?;
    }
}
```

| Category | Codes | Delay | Action |
|----------|-------|-------|--------|
| Permanent | 0, 1, 2, 6, 9, 17, 18 | -- | Do NOT reconnect |
| Rate-limited | 12 | 130 seconds | Wait full cooldown |
| Transient | All others | 2 seconds | Reconnect |

See [Reconnection & Error Handling](./streaming/reconnection) for complete reconnection examples across all SDKs.

## Event Reference

### Data Events

Every data event carries `received_at_ns` (wall-clock nanoseconds since UNIX epoch).

| Event | Key Fields | Notes |
|-------|------------|-------|
| `Quote` | contract_id, ms_of_day, bid, ask, bid_size, ask_size, bid_exchange, ask_exchange, bid_condition, ask_condition, price_type, date, received_at_ns | 11 fields + received_at_ns |
| `Trade` | contract_id, ms_of_day, sequence, ext_condition1-4, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, price_type, date, received_at_ns | 16 fields + received_at_ns. Dev server sends 8-field format (handled transparently). |
| `OpenInterest` | contract_id, ms_of_day, open_interest, date, received_at_ns | 3 fields + received_at_ns |
| `Ohlcvc` | contract_id, ms_of_day, open, high, low, close, volume (i64), count (i64), price_type, date, received_at_ns | volume/count are i64 to avoid overflow |

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

### RawData

Undecoded fallback for corrupt or unrecognized frames. Fields: `code` (u8), `payload` (bytes).

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
| `unsubscribe_full_trades(sec_type)` | Unsubscribe from all trades |
| `unsubscribe_full_open_interest(sec_type)` | Unsubscribe from all OI |
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
| `Close` | `()` | Free the FPSS handle |

Helper: `PriceToF64(value int32, priceType int32) float64` -- decode raw integer prices. Note: FPSS event price fields are pre-decoded to `float64` as of v5.2; this helper is for custom use cases or raw field decoding.

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
| `next_event` | `(timeout_ms) -> FpssEventPtr` | Poll next event (nullptr on timeout). RAII: auto-freed. |
| `is_authenticated` | `() -> bool` | Check FPSS auth status |
| `contract_lookup` | `(id) -> std::optional<std::string>` | Look up contract by server-assigned ID |
| `active_subscriptions` | `() -> std::string` | Get active subscriptions |
| `shutdown` | `() -> void` | Graceful shutdown |

Helper: `tdx::price_to_f64(int32_t value, int32_t price_type) -> double`

## Detailed Documentation

- [Connecting & Subscribing](./streaming/connection) -- server environments, flush mode, custom hosts, subscription management
- [Handling Events](./streaming/events) -- complete field reference tables for all event types, SDK-specific representations
- [Latency Measurement](./streaming/latency) -- `received_at_ns`, `tdbe::latency::latency_ns()`, lowest-latency configuration
- [Reconnection & Error Handling](./streaming/reconnection) -- `reconnect_streaming()`, disconnect categories, complete examples
