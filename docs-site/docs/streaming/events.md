---
title: Handling Events
description: Process data and control events from the FPSS streaming connection - quotes, trades, open interest, OHLC, and control messages.
---

# Handling Events

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

## Event Reference

### Data Events

<div class="param-list">
<div class="param">
<div class="param-header"><code>Quote</code><span class="param-type">data</span></div>
<div class="param-desc">Real-time NBBO quote update. Fields: <code>contract_id</code>, <code>ms_of_day</code>, <code>bid</code>, <code>ask</code>, <code>bid_size</code>, <code>ask_size</code>, <code>price_type</code>, <code>date</code></div>
</div>
<div class="param">
<div class="param-header"><code>Trade</code><span class="param-type">data</span></div>
<div class="param-desc">Individual trade execution. Fields: <code>contract_id</code>, <code>ms_of_day</code>, <code>price</code>, <code>size</code>, <code>exchange</code>, <code>condition</code>, <code>price_type</code>, <code>date</code></div>
</div>
<div class="param">
<div class="param-header"><code>OpenInterest</code><span class="param-type">data</span></div>
<div class="param-desc">Current open interest snapshot. Fields: <code>contract_id</code>, <code>ms_of_day</code>, <code>open_interest</code>, <code>date</code></div>
</div>
<div class="param">
<div class="param-header"><code>Ohlcvc</code><span class="param-type">data</span></div>
<div class="param-desc">Aggregated OHLC bar with volume and trade count. Fields: <code>contract_id</code>, <code>ms_of_day</code>, <code>open</code>, <code>high</code>, <code>low</code>, <code>close</code>, <code>volume</code>, <code>count</code>, <code>price_type</code>, <code>date</code></div>
</div>
</div>

### Control Events

<div class="param-list">
<div class="param">
<div class="param-header"><code>LoginSuccess</code><span class="param-type">control</span></div>
<div class="param-desc">Authentication succeeded. Fields: <code>permissions</code> (string)</div>
</div>
<div class="param">
<div class="param-header"><code>ContractAssigned</code><span class="param-type">control</span></div>
<div class="param-desc">Server assigned an integer ID to a subscribed contract. Fields: <code>id</code>, <code>contract</code></div>
</div>
<div class="param">
<div class="param-header"><code>ReqResponse</code><span class="param-type">control</span></div>
<div class="param-desc">Response to a subscribe/unsubscribe request. Fields: <code>req_id</code>, <code>result</code> (<code>Subscribed</code> / <code>Error</code> / <code>MaxStreamsReached</code> / <code>InvalidPerms</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>MarketOpen</code><span class="param-type">control</span></div>
<div class="param-desc">Market has opened for the trading day. No additional fields.</div>
</div>
<div class="param">
<div class="param-header"><code>MarketClose</code><span class="param-type">control</span></div>
<div class="param-desc">Market has closed for the trading day. No additional fields.</div>
</div>
<div class="param">
<div class="param-header"><code>ServerError</code><span class="param-type">control</span></div>
<div class="param-desc">Non-fatal server error. Fields: <code>message</code> (string)</div>
</div>
<div class="param">
<div class="param-header"><code>Disconnected</code><span class="param-type">control</span></div>
<div class="param-desc">Connection was terminated. Fields: <code>reason</code> (<code>RemoveReason</code> enum)</div>
</div>
<div class="param">
<div class="param-header"><code>Error</code><span class="param-type">control</span></div>
<div class="param-desc">Generic error event. Fields: <code>message</code> (string)</div>
</div>
</div>

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
| `unsubscribe_quotes` | `(symbol) -> int32_t` | Unsubscribe from quotes |
| `unsubscribe_trades` | `(symbol) -> int32_t` | Unsubscribe from trades |
| `unsubscribe_open_interest` | `(symbol) -> int32_t` | Unsubscribe from OI |
| `next_event` | `(timeout_ms) -> std::string` | Poll next event (empty on timeout) |
| `is_authenticated` | `() -> bool` | Check FPSS auth status |
| `contract_lookup` | `(id) -> std::optional<std::string>` | Look up contract by server-assigned ID |
| `active_subscriptions` | `() -> std::string` | Get active subscriptions as JSON |
| `shutdown` | `() -> void` | Graceful shutdown |
