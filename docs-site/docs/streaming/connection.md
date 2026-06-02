---
title: Connecting & Subscribing
description: Establish a streaming connection to streaming, choose server environments, configure flush mode, subscribe to quotes, trades, and open interest, and manage subscriptions.
---

# Connecting & Subscribing

## Server Environments

ThetaDataDx supports three streaming server environments:

| Config | Ports | Use case |
|--------|-------|----------|
| `DirectConfig::production()` | 20000/20001 | Live market data (NJ-A and NJ-B hosts) |
| `DirectConfig::dev()` | 20200/20201 | Replays a random historical trading day in an infinite loop at max speed. Use when markets are closed. |
| `DirectConfig::stage()` | 20100/20101 | Testing/staging servers. Frequent reboots, not stable. |

All three share the same historical-channel production servers -- only streaming hosts differ.

## TLS & SPKI Pinning

The streaming channel uses SPKI (Subject Public Key Info) pinning via a constant-time SHA-256 comparison against the captured ThetaData keypair. The pin survives cert renewal as long as ThetaData keeps the keypair; key rotation requires a coordinated client update. MITM attacks presenting a different certificate (even a valid CA-signed one) are rejected with `RustlsError::General("streaming SPKI pin mismatch ...")`.

## Connect (Production)

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let client = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

client.start_streaming(|event: &FpssEvent| {
    match event {
        // Every data event carries an `Arc<Contract>` — read the symbol
        // directly, no contract-ID map lookup required.
        FpssEvent::Data(FpssData::Quote { contract, bid, ask, received_at_ns, .. }) => {
            println!("Quote: {} bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns", contract.symbol);
        }
        FpssEvent::Data(FpssData::Trade { contract, price, size, received_at_ns, .. }) => {
            println!("Trade: {} price={price:.2} size={size} rx={received_at_ns}ns", contract.symbol);
        }
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
            println!("Contract {id} = {contract}");
        }
        _ => {}
    }
})?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
client = ThetaDataDxClient(creds, Config.production())

client.start_streaming(lambda event: print(event))
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto config = tdx::Config::production();
auto client = tdx::UnifiedClient::connect(creds, config);
```
```typescript [TypeScript]
import { ThetaDataDxClient } from 'thetadatadx';

const client = await ThetaDataDxClient.connectFromFile('creds.txt');
client.startStreaming((event) => console.log(event));
```
:::

::: tip
Every binding registers a push callback on the unified `ThetaDataDxClient`: `start_streaming(callback)` in Rust/Python, `startStreaming(callback)` in TypeScript, `set_callback(lambda)` in C++. The Disruptor consumer thread invokes the callback for every typed event under `catch_unwind`.
:::

## Connect (Dev Server)

The dev server replays historical data at maximum speed -- ideal for testing when markets are closed.

::: code-group
```rust [Rust]
let client = ThetaDataDxClient::connect(&creds, DirectConfig::dev()).await?;
```
```python [Python]
client = ThetaDataDxClient(creds, Config.dev())
```
```cpp [C++]
auto config = tdx::Config::dev();
auto client = tdx::UnifiedClient::connect(creds, config);
```
```typescript [TypeScript]
// Dev server config is not yet exposed in the TypeScript SDK.
// connectFromFile() always uses production config.
const client = await ThetaDataDxClient.connectFromFile('creds.txt');
```
:::

::: info Dev server trade format
The dev server sends a simplified 8-field trade format instead of the full 16-field production format. This is handled transparently by the SDK -- your code sees the same `Trade` event type with missing fields zeroed out.
:::

## Connect (Stage Server)

::: code-group
```rust [Rust]
let client = ThetaDataDxClient::connect(&creds, DirectConfig::stage()).await?;
```
```python [Python]
client = ThetaDataDxClient(creds, Config.stage())
```
```cpp [C++]
auto config = tdx::Config::stage();
auto client = tdx::UnifiedClient::connect(creds, config);
```
```typescript [TypeScript]
// Stage server config is not yet exposed in the TypeScript SDK.
// connectFromFile() always uses production config.
const client = await ThetaDataDxClient.connectFromFile('creds.txt');
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
let client = ThetaDataDxClient::connect(&creds, config).await?;
```
```python [Python]
# Flush mode cannot currently be changed from the Python SDK.
# It defaults to Batched (flush on PING frames, ~100ms).
# Use the Rust SDK directly if you need Immediate mode.
client = ThetaDataDxClient(creds, Config.production())
```
```cpp [C++]
auto config = tdx::Config::production();
config.set_flush_mode(tdx::FlushMode::Immediate);
auto client = tdx::UnifiedClient::connect(creds, config);
```
```typescript [TypeScript]
// Flush mode cannot currently be changed from the TypeScript SDK.
// It defaults to Batched (flush on PING frames, ~100ms).
const client = await ThetaDataDxClient.connectFromFile('creds.txt');
```
:::

## Custom streaming Hosts

Streaming hosts are not hardcoded. You can override them:

::: code-group
```rust [Rust]
let mut config = DirectConfig::production();
config.fpss_hosts = vec![
    ("custom-host-a.example.com".to_string(), 20000),
    ("custom-host-b.example.com".to_string(), 20000),
];
let client = ThetaDataDxClient::connect(&creds, config).await?;

// Or parse from a comma-separated string (same format as config_0.properties):
let hosts = DirectConfig::parse_fpss_hosts("host-a:20000,host-b:20001")?;
```
```python [Python]
# Custom hosts are configured at the Rust level via DirectConfig or
# TOML config file. Python inherits them from the config at connection time.
# Set hosts in config.toml:
#   [fpss]
#   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
client = ThetaDataDxClient(creds, Config.production())
```
```cpp [C++]
// Custom hosts are configured at the Rust level via DirectConfig or
// TOML config file. C++ inherits them from the config at connection time.
// Set hosts in config.toml:
//   [fpss]
//   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
auto config = tdx::Config::production();
auto client = tdx::UnifiedClient::connect(creds, config);
```
```typescript [TypeScript]
// Custom hosts are configured at the Rust level via DirectConfig or
// TOML config file. TypeScript inherits them from the config at connection time.
// Set hosts in config.toml:
//   [fpss]
//   hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
const client = await ThetaDataDxClient.connectFromFile('creds.txt');
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
| `start_streaming()` + callbacks | **sync** (OS threads) | Dedicated I/O thread + ring buffer for lowest latency |

**What this means for your code:**

- You need a tokio runtime for `connect()` and any historical data call (`stock_history_eod`, etc.).
- The streaming callback (`FnMut(&FpssEvent)`) runs on a plain OS thread -- no async executor involved. This eliminates all executor scheduling jitter from the hot path.
- `subscribe()` and `unsubscribe()` are synchronous — they send a command through an internal channel to the I/O thread.

```text
tokio runtime
  +-- connect()          async, gRPC/tonic/HTTP2
  +-- stock_history_*()  async, gRPC streaming

std::thread (fpss-io)
  +-- TLS read loop      blocking, 50ms timeout
  +-- ring publish  lock-free, zero-alloc

std::thread (fpss-ping)
  +-- PING heartbeat     100ms sleep loop

ring-buffer consumer thread
  +-- your callback(FnMut(&FpssEvent))
```

You can safely call `subscribe(spec)` / `unsubscribe(spec)` from any thread -- the command is sent through an `mpsc` channel and executed by the I/O thread.

## Subscribe

Every binding exposes a single polymorphic `subscribe()` method on the unified `ThetaDataDxClient`. Build a typed subscription spec by calling a topic helper — `quote()`, `trade()`, `open_interest()` — on a `Contract`, or `full_trades()` / `full_open_interest()` on a `SecType`, and hand the result to `subscribe()`.

::: code-group
```rust [Rust]
// Stock quotes
client.subscribe(Contract::stock("AAPL").quote())?;

// Stock trades
client.subscribe(Contract::stock("MSFT").trade())?;

// Quotes + trades in one call
client.subscribe(Contract::stock("TSLA").all())?;

// Option quotes
let opt = Contract::option("SPY", "20261218", "600", "C")?;
client.subscribe(opt.quote())?;

// Open interest
client.subscribe(Contract::stock("AAPL").open_interest())?;

// All trades for a security type (full-stream subscription)
client.subscribe(SecType::Stock.full_trades())?;

// All open interest for a security type (full-stream subscription)
client.subscribe(SecType::Option.full_open_interest())?;
```
```python [Python]
client.subscribe(Contract.stock("AAPL").quote())
client.subscribe(Contract.stock("MSFT").trade())
client.subscribe(Contract.stock("SPY").open_interest())
client.subscribe(SecType.Stock.full_trades())
client.subscribe(SecType.Option.full_open_interest())
```
```cpp [C++]
// Stock quotes
client.subscribe(tdx::Contract::stock("AAPL").quote());

// Stock trades
client.subscribe(tdx::Contract::stock("MSFT").trade());

// Open interest
client.subscribe(tdx::Contract::stock("AAPL").open_interest());

// All trades for a security type (full-stream subscription)
client.subscribe(tdx::SecType::Stock.full_trades());

// All open interest for a security type
client.subscribe(tdx::SecType::Option.full_open_interest());
```
```typescript [TypeScript]
// Stock quotes
client.subscribe(Contract.stock('AAPL').quote());

// Stock trades
client.subscribe(Contract.stock('MSFT').trade());

// Open interest
client.subscribe(Contract.stock('AAPL').openInterest());

// All trades for a security type (full-stream subscription)
client.subscribe(SecType.Stock.fullTrades());

// All open interest for a security type
client.subscribe(SecType.Option.fullOpenInterest());
```
:::

The full-stream subscription (`full_trades` / `full_open_interest`) is available for the Stock and Option security types only; indices and rates have no full-stream broadcast upstream and must be subscribed to per-contract (for example `Contract::index("VIX").trade()`). A full-stream subscription on any other security type is rejected with a configuration error at `subscribe` time.

## Typed Contract on Data Events

streaming assigns integer IDs to contracts on the wire, but the SDK resolves every data event's contract before user code sees it. `Quote` / `Trade` / `OpenInterest` / `Ohlcvc` events carry a typed contract with `symbol`, `sec_type`, `expiration`, `strike` / `strike_dollars`, and the option side. Each field surfaces in the language-idiomatic shape:

- **Rust**: `Contract { symbol: String, sec_type: SecType, expiration: Option<i32>, is_call: Option<bool>, strike: Option<i32> }` plus the derived accessors `right() -> Option<Right>` and `strike_dollars() -> Option<f64>`.
- **Python**: `event.contract` exposes `symbol: str`, `sec_type: str` (`"STOCK"` / `"OPTION"` / `"INDEX"` / `"RATE"`), `expiration: Optional[int]`, `right: Optional[str]` (`"C"` / `"P"`), `strike_dollars: Optional[float]`, and the wire-level `strike: Optional[int]`.
- **TypeScript**: same field set as Python (`secType: string`, `right: string | null`, `strikeDollars: number | null`, `strike: number | null`).
- **C / C++**: `TdxContract { const char* symbol; int32_t sec_type; bool has_expiration; int32_t expiration; bool has_right; char right; bool has_strike; int32_t strike; }`. Strike stays in the wire integer form on the C ABI for layout stability; convert to dollars with `strike / 1000.0` (or use the `tdx::Contract` C++ wrapper's accessors).

User code reads the symbol directly off the event without a side-table lookup.

::: code-group
```rust [Rust]
client.start_streaming(|event: &FpssEvent| {
    match event {
        // Read the typed contract directly off the event; no integer
        // ID lookup required.
        FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
            println!("{}: bid={bid:.2} ask={ask:.2}", contract.symbol);
        }
        FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
            println!("{}: price={price:.2} size={size}", contract.symbol);
        }
        _ => {}
    }
})?;

// Snapshot active subscriptions — useful for diagnostics dashboards.
let subs = client.active_subscriptions()?;
for (kind, contract) in subs {
    println!("  {kind:?}: {}", contract.symbol);
}
```
```python [Python]
# Push-callback delivery: read `event.contract.symbol` directly off
# the typed event.
def on_event(event):
    if event.kind == "quote":
        print(f"[QUOTE] {event.contract.symbol}: bid={event.bid} ask={event.ask}")
    elif event.kind == "trade":
        print(f"[TRADE] {event.contract.symbol}: price={event.price} size={event.size}")

client.start_streaming(on_event)

# Snapshot active subscriptions.
for sub in client.active_subscriptions():
    print(sub)
```
```cpp [C++]
// Each event carries event.quote.contract.symbol etc. directly —
// `has_expiration` / `has_right` / `has_strike` gate the
// option-only fields. Register a callback on the unified client:
client.set_callback([](const tdx::FpssEvent& event) {
    if (event.kind == TDX_FPSS_QUOTE) {
        auto& q = event.quote;
        std::cout << "[QUOTE] " << q.contract.symbol
                  << ": bid=" << q.bid << " ask=" << q.ask << std::endl;
    }
});

// Snapshot active subscriptions.
auto subs = client.active_subscriptions();
for (const auto& sub : subs) {
    std::cout << "  " << static_cast<int>(sub.kind)
              << ": " << sub.contract.symbol << std::endl;
}
```
```typescript [TypeScript]
// Each event carries the resolved typed contract directly — read
// event.contract.symbol off the event in your callback or async
// iterator loop, no side-table lookup required.

// Snapshot active subscriptions.
const subs = client.activeSubscriptions();
```
:::

## Unsubscribe

Unsubscribe takes the same typed subscription spec as `subscribe()` — pass the matching `quote()` / `trade()` / `open_interest()` / `full_trades()` / `full_open_interest()` topic.

::: code-group
```rust [Rust]
client.unsubscribe(Contract::stock("AAPL").quote())?;
client.unsubscribe(Contract::stock("MSFT").trade())?;
client.unsubscribe(Contract::stock("AAPL").open_interest())?;
client.unsubscribe(SecType::Stock.full_trades())?;
client.unsubscribe(SecType::Option.full_open_interest())?;
```
```python [Python]
client.unsubscribe(Contract.stock("AAPL").quote())
client.unsubscribe(Contract.stock("MSFT").trade())
client.unsubscribe(Contract.stock("SPY").open_interest())
client.unsubscribe(SecType.Stock.full_trades())
client.unsubscribe(SecType.Option.full_open_interest())
```
```cpp [C++]
client.unsubscribe(tdx::Contract::stock("AAPL").quote());
client.unsubscribe(tdx::Contract::stock("MSFT").trade());
client.unsubscribe(tdx::Contract::stock("AAPL").open_interest());
client.unsubscribe(tdx::SecType::Stock.full_trades());
client.unsubscribe(tdx::SecType::Option.full_open_interest());
```
```typescript [TypeScript]
client.unsubscribe(Contract.stock('AAPL').quote());
client.unsubscribe(Contract.stock('MSFT').trade());
client.unsubscribe(Contract.stock('AAPL').openInterest());
client.unsubscribe(SecType.Stock.fullTrades());
client.unsubscribe(SecType.Option.fullOpenInterest());
```
:::

## Stop Streaming

::: code-group
```rust [Rust]
client.stop_streaming();
```
```python [Python]
client.stop_streaming()
```
```cpp [C++]
client.stop_streaming();
// RAII also handles cleanup: the ThetaDataDxClient destructor stops streaming on drop.
```
```typescript [TypeScript]
client.stopStreaming();
```
:::
