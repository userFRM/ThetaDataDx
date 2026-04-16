---
title: Connecting & Subscribing
description: Establish a streaming connection to FPSS, choose server environments, configure flush mode, subscribe to quotes, trades, and open interest, and manage subscriptions.
---

# Connecting & Subscribing

## Server Environments

ThetaDataDx supports three FPSS server environments:

| Config | Ports | Use case |
|--------|-------|----------|
| `DirectConfig::production()` | 20000/20001 | Live market data (NJ-A and NJ-B hosts) |
| `DirectConfig::dev()` | 20200/20201 | Replays a random historical trading day in an infinite loop at max speed. Use when markets are closed. |
| `DirectConfig::stage()` | 20100/20101 | Testing/staging servers. Frequent reboots, not stable. |

All three share the same MDDS (historical) production servers -- only FPSS hosts differ.

## Connect (Production)

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, received_at_ns, .. }) => {
            println!("Quote: contract={contract_id} bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns");
        }
        FpssEvent::Data(FpssData::Trade { contract_id, price, size, received_at_ns, .. }) => {
            println!("Trade: contract={contract_id} price={price:.2} size={size} rx={received_at_ns}ns");
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
The Rust SDK uses a callback model where you provide a closure to `start_streaming`. Python, Go, and C++ use a polling model where you call `next_event()` / `NextEvent()` in a loop.
:::

## Connect (Dev Server)

The dev server replays historical data at maximum speed -- ideal for testing when markets are closed.

::: code-group
```rust [Rust]
let tdx = ThetaDataDx::connect(&creds, DirectConfig::dev()).await?;
```
```python [Python]
tdx = ThetaDataDx(creds, Config.dev())
```
```go [Go]
config := thetadatadx.DevConfig()
defer config.Close()
fpss, _ := thetadatadx.NewFpssClient(creds, config)
```
```cpp [C++]
auto config = tdx::Config::dev();
tdx::FpssClient fpss(creds, config);
```
:::

::: info Dev server trade format
The dev server sends a simplified 8-field trade format instead of the full 16-field production format. This is handled transparently by the SDK -- your code sees the same `Trade` event type with missing fields zeroed out.
:::

## Connect (Stage Server)

::: code-group
```rust [Rust]
let tdx = ThetaDataDx::connect(&creds, DirectConfig::stage()).await?;
```
```python [Python]
tdx = ThetaDataDx(creds, Config.stage())
```
```go [Go]
config := thetadatadx.StageConfig()
defer config.Close()
fpss, _ := thetadatadx.NewFpssClient(creds, config)
```
```cpp [C++]
auto config = tdx::Config::stage();
tdx::FpssClient fpss(creds, config);
```
:::

## Flush Mode

`FpssFlushMode` controls the latency/syscall tradeoff on the write path:

| Mode | Flush trigger | Added latency | Best for |
|------|--------------|---------------|----------|
| `Batched` (default) | PING frames every ~100ms | Up to 100ms | Production throughput, matches Java terminal |
| `Immediate` | Every frame write | None | Lowest latency trading |

::: code-group
```rust [Rust]
use thetadatadx::config::FpssFlushMode;

let mut config = DirectConfig::production();
config.fpss_flush_mode = FpssFlushMode::Immediate; // lowest latency
let tdx = ThetaDataDx::connect(&creds, config).await?;
```
```python [Python]
# Flush mode cannot currently be changed from the Python SDK.
# It defaults to Batched (flush on PING frames, ~100ms).
# Use the Rust SDK directly if you need Immediate mode.
tdx = ThetaDataDx(creds, Config.production())
```
```go [Go]
config := thetadatadx.ProductionConfig()
config.SetFlushMode(thetadatadx.FlushModeImmediate)
defer config.Close()
fpss, _ := thetadatadx.NewFpssClient(creds, config)
```
```cpp [C++]
auto config = tdx::Config::production();
config.set_flush_mode(tdx::FlushMode::Immediate);
tdx::FpssClient fpss(creds, config);
```
:::

## Custom FPSS Hosts

FPSS hosts are not hardcoded. You can override them:

::: code-group
```rust [Rust]
let mut config = DirectConfig::production();
config.fpss_hosts = vec![
    ("custom-host-a.example.com".to_string(), 20000),
    ("custom-host-b.example.com".to_string(), 20000),
];
let tdx = ThetaDataDx::connect(&creds, config).await?;

// Or parse from a comma-separated string (same format as config_0.properties):
let hosts = DirectConfig::parse_fpss_hosts("host-a:20000,host-b:20001")?;
```
```python [Python]
# Custom hosts are configured at the Rust level via DirectConfig or
# TOML config file. Python inherits them from the config at connection time.
# Set hosts in config.toml:
#   [fpss]
#   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
tdx = ThetaDataDx(creds, Config.production())
```
```go [Go]
// Custom hosts are configured at the Rust level via DirectConfig or
// TOML config file. Go inherits them from the config at connection time.
// Set hosts in config.toml:
//   [fpss]
//   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
config := thetadatadx.ProductionConfig()
defer config.Close()
fpss, _ := thetadatadx.NewFpssClient(creds, config)
```
```cpp [C++]
// Custom hosts are configured at the Rust level via DirectConfig or
// TOML config file. C++ inherits them from the config at connection time.
// Set hosts in config.toml:
//   [fpss]
//   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
auto config = tdx::Config::production();
tdx::FpssClient fpss(creds, config);
```
:::

Or use a TOML config file (requires `config-file` feature):

```toml
[fpss]
hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
# Or as CSV:
# hosts = "host-a.example.com:20000,host-b.example.com:20001"
```

## Async/Sync Design

ThetaDataDx uses two different concurrency models for its two data paths:

| Path | Runtime | Why |
|------|---------|-----|
| `connect()` + all historical methods | **async** (tokio) | gRPC/tonic requires tokio for HTTP/2 multiplexing |
| `start_streaming()` + callbacks | **sync** (OS threads) | Dedicated I/O thread + LMAX Disruptor ring buffer for lowest latency |

**What this means for your code:**

- You need a tokio runtime for `connect()` and any historical data call (`stock_history_eod`, etc.).
- The streaming callback (`FnMut(&FpssEvent)`) runs on a plain OS thread -- no async executor involved. This eliminates all executor scheduling jitter from the hot path.
- `subscribe_quotes()` and other subscription methods are synchronous -- they send a command through an internal channel to the I/O thread.

```text
tokio runtime
  +-- connect()          async, gRPC/tonic/HTTP2
  +-- stock_history_*()  async, gRPC streaming

std::thread (fpss-io)
  +-- TLS read loop      blocking, 50ms timeout
  +-- Disruptor publish  lock-free, zero-alloc

std::thread (fpss-ping)
  +-- PING heartbeat     100ms sleep loop

Disruptor consumer thread
  +-- your callback(FnMut(&FpssEvent))
```

You can safely call `subscribe_*()` from any thread -- the command is sent through an `mpsc` channel and executed by the I/O thread.

## Subscribe

::: code-group
```rust [Rust]
// Stock quotes
tdx.subscribe_quotes(&Contract::stock("AAPL"))?;

// Stock trades
tdx.subscribe_trades(&Contract::stock("MSFT"))?;

// Quotes + trades in one call
tdx.subscribe_all(&Contract::stock("TSLA"))?;

// Option quotes
let opt = Contract::option("SPY", "20261218", "600", "C")?;
tdx.subscribe_quotes(&opt)?;

// Open interest
tdx.subscribe_open_interest(&Contract::stock("AAPL"))?;

// All trades for a security type (firehose)
tdx.subscribe_full_trades(SecType::Stock)?;

// All open interest for a security type (firehose)
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
// Stock quotes
_, err := fpss.SubscribeQuotes("AAPL")

// Stock trades
fpss.SubscribeTrades("MSFT")

// Open interest
fpss.SubscribeOpenInterest("AAPL")

// All trades for a security type (firehose)
fpss.SubscribeFullTrades("STOCK")

// All open interest for a security type
fpss.SubscribeFullOpenInterest("OPTION")
```
```cpp [C++]
// Stock quotes (returns 0 on success, -1 on error)
fpss.subscribe_quotes("AAPL");

// Stock trades
fpss.subscribe_trades("MSFT");

// Open interest
fpss.subscribe_open_interest("AAPL");

// All trades for a security type (firehose)
fpss.subscribe_full_trades("STOCK");

// All open interest for a security type
fpss.subscribe_full_open_interest("OPTION");
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
        FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => {
            if let Some(contract) = contracts_clone.lock().unwrap().get(contract_id) {
                println!("{}: bid={bid:.2} ask={ask:.2}", contract.root);
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

// List all active subscriptions as typed structs
subs, _ := fpss.ActiveSubscriptions()
for _, s := range subs {
    fmt.Printf("  %s: %s\n", s.Kind, s.Contract)
}
```
```cpp [C++]
// Look up a contract by its server-assigned ID
auto contract = fpss.contract_lookup(42);
if (contract.has_value()) {
    std::cout << "Contract: " << contract.value() << std::endl;
}

// List all active subscriptions
auto subs = fpss.active_subscriptions();
for (const auto& sub : subs) {
    std::cout << "  " << sub.kind << ": " << sub.contract << std::endl;
}
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
