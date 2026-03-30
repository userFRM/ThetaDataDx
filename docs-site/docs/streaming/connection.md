---
title: Connecting & Subscribing
description: Establish a streaming connection to FPSS, subscribe to quotes, trades, and open interest, and manage subscriptions.
---

# Connecting & Subscribing

## Connect

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

tdx.start_streaming(|event: &FpssEvent| {
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
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

fpss, _ := thetadatadx.NewFpssClient(creds, config)
defer fpss.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto config = tdx::Config::production();
tdx::FpssClient fpss(creds, config);
```
:::

::: tip
The Rust SDK uses a callback model where you provide a closure to `start_streaming`. Python, Go, and C++ use a polling model where you call `next_event()` in a loop.
:::

The ring buffer size for event dispatch is configured via `DirectConfig` (Rust only).

## Subscribe

::: code-group
```rust [Rust]
// Stock quotes
let req_id = tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
println!("Subscribed (req_id={req_id})");

// Stock trades
tdx.subscribe_trades(&Contract::stock("MSFT"))?;

// Option quotes
let opt = Contract::option("SPY", 20261218, true, 60000); // call, strike $600
tdx.subscribe_quotes(&opt)?;

// Open interest
tdx.subscribe_open_interest(&Contract::stock("AAPL"))?;

// All trades for a security type
tdx.subscribe_full_trades(SecType::Stock)?;
```
```python [Python]
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")
tdx.subscribe_open_interest("SPY")
```
```go [Go]
// Stock quotes
reqID, _ := fpss.SubscribeQuotes("AAPL")
fmt.Printf("Subscribed (req_id=%d)\n", reqID)

// Stock trades
fpss.SubscribeTrades("MSFT")

// Open interest
fpss.SubscribeOpenInterest("AAPL")

// All trades for a security type
fpss.SubscribeFullTrades("STOCK")
```
```cpp [C++]
// Stock quotes
int32_t req_id = fpss.subscribe_quotes("AAPL");
std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

// Stock trades
fpss.subscribe_trades("MSFT");

// Open interest
fpss.subscribe_open_interest("AAPL");

// All trades for a security type
fpss.subscribe_full_trades("STOCK");
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
# Build a mapping as events arrive
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
// Look up a contract by its server-assigned ID
contract, err := fpss.ContractLookup(42)
if err != nil {
    log.Fatal(err)
}
fmt.Println("Contract:", contract)

// List all active subscriptions
subs, _ := fpss.ActiveSubscriptions()
fmt.Println("Active:", string(subs))
```
```cpp [C++]
// Look up a contract by its server-assigned ID
auto contract = fpss.contract_lookup(42);
if (contract.has_value()) {
    std::cout << "Contract: " << contract.value() << std::endl;
}

// List all active subscriptions
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
```
```python [Python]
tdx.unsubscribe_quotes("AAPL")
tdx.unsubscribe_trades("MSFT")
tdx.unsubscribe_open_interest("SPY")
```
```go [Go]
fpss.UnsubscribeQuotes("AAPL")
fpss.UnsubscribeTrades("MSFT")
fpss.UnsubscribeOpenInterest("AAPL")
```
```cpp [C++]
fpss.unsubscribe_quotes("AAPL");
fpss.unsubscribe_trades("MSFT");
fpss.unsubscribe_open_interest("AAPL");
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
