# Real-Time Streaming (Rust)

Real-time market data via ThetaData's FPSS (Feed Protocol Streaming Service) servers. FPSS delivers live quotes, trades, open interest, and OHLC snapshots over a persistent TLS/TCP connection.

The Rust FPSS client is fully synchronous -- no Tokio on the streaming hot path. Events are dispatched through an LMAX Disruptor ring buffer via a sync callback.

## Connect

```rust
use thetadatadx::auth::Credentials;
use thetadatadx::fpss::{FpssClient, FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;

let client = FpssClient::connect(&creds, 1024, |event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
            println!("Quote: contract={contract_id} bid={bid} ask={ask}");
        }
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
            println!("Trade: contract={contract_id} price={price} size={size}");
        }
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
            println!("Contract {id} = {contract}");
        }
        _ => {}
    }
})?;
```

The `1024` parameter is the ring buffer size for event dispatch.

### Without OHLCVC Derivation

If you only need raw trades/quotes and want lower overhead:

```rust
let client = FpssClient::connect_no_ohlcvc(&creds, 1024, |event: &FpssEvent| {
    // No FpssData::Ohlcvc events will be emitted
    match event {
        FpssEvent::Data(FpssData::Trade { .. }) => { /* ... */ }
        _ => {}
    }
})?;
```

## Subscribe

```rust
// Stock quotes
let req_id = client.subscribe_quotes(&Contract::stock("AAPL"))?;
println!("Subscribed (req_id={req_id})");

// Stock trades
client.subscribe_trades(&Contract::stock("MSFT"))?;

// Option quotes
let opt = Contract::option("SPY", 20261218, true, 60000); // call, strike $600
client.subscribe_quotes(&opt)?;

// Open interest
client.subscribe_open_interest(&Contract::stock("AAPL"))?;

// All trades for a security type
client.subscribe_full_trades(SecType::Stock)?;
```

## Receive Events

The callback fires on the ring buffer's consumer thread. The v2.0.0 `FpssEvent` is split into `FpssData` and `FpssControl`:

```rust
let client = FpssClient::connect(&creds, 1024, |event: &FpssEvent| {
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

## Contract ID Mapping

FPSS assigns integer IDs to contracts. Use `ContractAssigned` events to build a mapping:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

let contracts: Arc<Mutex<HashMap<i32, Contract>>> = Arc::new(Mutex::new(HashMap::new()));
let contracts_clone = contracts.clone();

let client = FpssClient::connect(&creds, 1024, move |event: &FpssEvent| {
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
```

Or use the built-in method:

```rust
let map: HashMap<i32, Contract> = client.contract_map();
```

## Unsubscribe

```rust
client.unsubscribe_quotes(&Contract::stock("AAPL"))?;
client.unsubscribe_trades(&Contract::stock("MSFT"))?;
client.unsubscribe_open_interest(&Contract::stock("AAPL"))?;
```

## Shutdown

```rust
client.shutdown()?;
```

## Reconnection

ThetaDataDx uses manual reconnection. When the server disconnects, you receive an `FpssControl::Disconnected` event with a reason code.

```rust
use thetadatadx::fpss::FpssClient;
use thetadatadx::types::RemoveReason;

match FpssClient::reconnect_delay(reason) {
    None => {
        // Permanent error (bad credentials, etc.) -- do NOT retry
        eprintln!("Permanent disconnect: {:?}", reason);
    }
    Some(delay_ms) => {
        // Reconnect after delay
        let (new_client, new_events) = FpssClient::reconnect(
            &creds,
            previous_subscriptions,
            delay_ms,
            1024,
        ).await?;
    }
}
```

### Disconnect Categories

| Category | Codes | Action |
|----------|-------|--------|
| Permanent | 0, 1, 2, 6, 9, 17, 18 | Do NOT reconnect |
| Rate-limited | 12 | Wait 130 seconds, then reconnect |
| Transient | All others | Wait 2 seconds, then reconnect |

## Event Reference

### Data Events (`FpssData`)

| Event | Key Fields |
|-------|------------|
| `Quote` | contract_id, ms_of_day, bid, ask, bid_size, ask_size, price_type, date |
| `Trade` | contract_id, ms_of_day, price, size, exchange, condition, price_type, date |
| `OpenInterest` | contract_id, ms_of_day, open_interest, date |
| `Ohlcvc` | contract_id, ms_of_day, open, high, low, close, volume, count, price_type, date |

### Control Events (`FpssControl`)

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
