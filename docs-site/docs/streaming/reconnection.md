---
title: Reconnection & Error Handling
description: Automatic streaming recovery with exponential backoff, jittered delays, paced re-subscription, connection watchdogs, and caller-driven reconnect APIs.
---

# Reconnection & Error Handling

## Automatic recovery

The streaming client recovers from involuntary disconnects on its own. With the default configuration it survives a multi-minute upstream outage unattended: it retries on an exponential schedule, spreads its retries so a fleet of clients does not reconnect in lockstep, restores every saved subscription at a paced cadence once a session is re-established, and emits a typed terminal event if it ever stops trying.

Disconnect reasons are classified into four classes, each with its own retry schedule and budget:

| Class | Reasons | Delay schedule | Budget |
|-------|---------|----------------|--------|
| **Permanent** | Bad credentials, account conflicts (all 7 credential/account codes) | — | No retries. Recovery stops immediately. |
| **Rate-limited** | `TooManyRequests` | 130 s floor + jitter window above it | 100 attempts (~3.6 h of sustained throttling) |
| **Server restart** | `ServerRestarting` | Flat 5 s, jittered | 60 attempts (~5 min pool-bounce window) |
| **Generic transient** | `TimedOut`, `Unspecified`, unknown codes | Exponential: 250 ms doubling to a 30 s cap, jittered | 30 attempts **or** a 5-minute wall-clock envelope, whichever first |

Unknown disconnect codes deliberately land in the generic-transient class — an unrecognised code is more likely transient than permanent, so the catch-all carries the long multi-minute budget.

Two details make the generic-transient schedule fleet-safe:

- **Jitter.** Every computed delay is jittered (`full` mode by default: uniform over `[0, delay]`), so a hundred clients dropped by the same upstream event scatter their retries instead of arriving back in one burst. The rate-limited floor is never jittered *below* — the cooldown is honoured in full and the jitter window sits on top of it.
- **Wall-clock envelope.** The attempt budget alone is hard to reason about in wall-clock terms once delays grow, so the envelope (`reconnect_max_elapsed_secs`, default 300) bounds a consecutive-reconnect sequence directly. Set it to `0` to disable and rely on attempt counts alone.

A session that runs cleanly for the stable window (default 60 s of received frames) earns its full retry budget back, so a brief blip at 10:00 does not reduce the budget available at 15:00. The window requires at least one received frame on the session — a sequence of connect-then-immediate-drop cycles keeps consuming the same budget rather than resetting it.

### Re-subscription is paced

After a successful reconnect, the client re-subscribes everything that was active — in bursts (default 50 frames per burst) with a short jittered pause between bursts (default 5 ms), rather than firing thousands of subscribe frames at a server that may itself be recovering. The same pacing applies when caller-driven reconnect APIs restore a saved subscription set.

### The terminal event

If recovery stops for any cause other than a user-initiated shutdown — budget or envelope exhaustion, a permanent disconnect reason, a `manual` policy, or a custom policy declining — the client publishes a terminal `ReconnectsExhausted` event carrying the final disconnect `reason` and the number of `attempts` consumed. Operators watching the event stream can distinguish "the stream gave up and needs intervention" from a clean `stop_streaming()`, which emits no terminal event because the caller initiated it.

```python
def on_event(event):
    if event.kind == "reconnects_exhausted":
        page_operator(f"stream gave up: {event.reason_name} after {event.attempts} attempts")
```

### Connection liveness

Three layers detect a dead connection, fastest first:

1. **TCP keepalive** — the socket is armed with an aggressive probe schedule (5 s idle / 2 s interval / 2 probes by default, ~9 s kernel-side detection of a peer that vanished without closing the connection).
2. **Read timeout** — no frame of any kind for `fpss_timeout_ms` (default 3 s; the server heartbeats every ~100 ms even on a quiet session) declares the session dead and triggers the reconnect engine.
3. **Last-frame watchdog** — a hard wall-clock backstop (`fpss_data_watchdog_ms`, default 30 s, `0` disables) above the read timeout, for deployments that widen it.

The clock behind the watchdog is public. `millis_since_last_event()` returns the staleness of the most recent inbound frame (`None` before streaming starts); `last_event_received_at_unix_nanos()` is the raw timestamp for pipelines that correlate against their own clocks; `last_connected_addr()` reports which server the live session is on, following it across reconnects.

```python
staleness = tdx.millis_since_last_event()
if staleness is not None and staleness > 5_000:
    log.warning("stream quiet for %dms on %s", staleness, tdx.last_connected_addr())
```

## Tuning the recovery engine

Every knob is exposed on `Config` across Rust, Python, TypeScript, C, and C++. Python names shown; TypeScript uses the camelCase setter form (`setReconnectWaitMaxMs(...)`), C uses `tdx_config_set_*`, C++ uses `set_*`.

| Knob | Default | Meaning |
|------|---------|---------|
| `reconnect_policy` | `"auto"` | `"auto"` (recover automatically), `"manual"` (never reconnect; caller drives), or `"custom"` (installed via the callback below) |
| `reconnect_wait_ms` | `250` | Initial delay of the generic-transient exponential ladder |
| `reconnect_wait_max_ms` | `30_000` | Cap on the ladder |
| `reconnect_wait_rate_limited_ms` | `130_000` | Rate-limited floor (jitter sits above it, never below) |
| `reconnect_wait_server_restart_ms` | `5_000` | Flat cadence for server-restart drops |
| `reconnect_jitter` | `"full"` | `"full"`, `"equal"`, `"decorrelated"`, or `"none"` (deterministic — tests only) |
| `reconnect_max_attempts` | `30` | Generic-transient attempt budget |
| `reconnect_max_elapsed_secs` | `300` | Wall-clock envelope for a consecutive-reconnect sequence; `0` disables |
| `reconnect_max_rate_limited_attempts` | `100` | Rate-limited attempt budget (exempt from the envelope) |
| `reconnect_max_server_restart_attempts` | `60` | Server-restart attempt budget |
| `reconnect_stable_window_secs` | `60` | Clean-runtime window after which budgets reset |
| `reconnect_replay_burst_size` | `50` | Subscribe frames per replay burst (minimum 1) |
| `reconnect_replay_pace_ms` | `5` | Jittered pause between replay bursts; `0` removes the pause |

Transport-level knobs:

| Knob | Default | Meaning |
|------|---------|---------|
| `fpss_timeout_ms` | `3_000` | No-frames deadline before the session is declared dead |
| `fpss_connect_timeout_ms` | `2_000` | Per-server connect timeout |
| `fpss_ping_interval_ms` | `250` | Client heartbeat cadence |
| `fpss_io_read_slice_ms` | `25` | I/O loop read slice (latency of outbound command service) |
| `fpss_data_watchdog_ms` | `30_000` | Last-frame watchdog; `0` disables |
| `fpss_keepalive_idle_secs` / `fpss_keepalive_interval_secs` / `fpss_keepalive_retries` | `5` / `2` / `2` | TCP keepalive schedule |
| `fpss_host_selection` | `"shuffled"` | `"shuffled"` spreads clients across hosts and makes consecutive failover attempts cross physical machines; `"fixed_order"` uses the declared order verbatim |
| `fpss_host_shuffle_seed` | `None` | Explicit seed makes the shuffled order deterministic (fleet sharding, tests) |

```python
from thetadatadx import Config

cfg = Config.production()
cfg.reconnect_max_elapsed_secs = 900   # ride out a 15-minute outage
cfg.reconnect_max_attempts = 120
cfg.fpss_data_watchdog_ms = 10_000     # tighter staleness backstop
```

### Custom reconnect policies

When the built-in classes do not fit, install a callback that receives `(reason, attempt)` for each retriable drop and returns the delay in milliseconds — or `None`/negative to stop (the terminal event then fires). Permanent reasons never reach the callback: no return value can turn a credential rejection into a retry loop.

::: code-group
```python [Python]
cfg = Config.production()
cfg.reconnect_callback = lambda reason, attempt: min(1_000 * attempt, 60_000)
```
```ts [TypeScript]
const cfg = Config.production();
cfg.setReconnectCallback(({ reason, attempt }) => Math.min(1_000 * attempt, 60_000));
```
```cpp [C++]
auto cfg = tdx::Config::production();
cfg.set_reconnect_callback(
    [](int32_t reason, uint32_t attempt, void*) -> int64_t {
        return std::min<int64_t>(1000 * attempt, 60000);
    },
    nullptr);
```
:::

The callback runs on (or is awaited by) the streaming I/O thread — return promptly, and treat it as running off your main thread.

## Caller-driven reconnection

Automatic recovery handles involuntary drops. The APIs below are for *caller-driven* session rebuilds — rotating credentials, applying a new subscription plan, or recovering after the terminal event.

Rust exposes `reconnect_streaming(handler)` on the unified `ThetaDataDxClient` client.
Python, TypeScript/Node.js, and C++ expose `reconnect()` on their public streaming clients.

### `reconnect_streaming()` (Rust)

The unified `ThetaDataDxClient` provides `reconnect_streaming()` which handles the full cycle:

1. Saves all active per-contract and full-stream subscriptions
2. Stops the current streaming connection
3. Starts a new streaming connection with your handler
4. Re-subscribes everything that was previously active (paced)

```rust
use thetadatadx::fpss::{FpssData, FpssEvent};

tdx.reconnect_streaming(|event: &FpssEvent| {
    if let FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) = event {
        println!("Quote: {} {bid:.2}/{ask:.2}", contract.symbol);
    }
})?;
```

A `restore_subscriptions(per_contract, full_type)` method is also public on both `ThetaDataDxClient` and `fpss::FpssClient` for flows that own their snapshot lifecycle — it is the same paced replay engine the automatic path uses, and returns `Error::PartialReconnect` listing anything that failed to restore.

::: tip
`reconnect_streaming()` uses the same `DirectConfig` (including the host list) that was passed at `ThetaDataDxClient::connect()` time. If hosts change, create a new `ThetaDataDxClient` instance.
:::

### `reconnect()` (Python, C++)

::: code-group
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

def on_event(event):
    print(event)

tdx.start_streaming(on_event)
tdx.subscribe(Contract.stock("AAPL").quote())
tdx.subscribe(Contract.option(
    "SPY", expiration="20260116", strike="600", right="C"
).quote())

# reconnect() restores the existing subscription set; the callback
# registered above is reused on the new session.
tdx.reconnect()
```
```cpp [C++]
#include "thetadx.hpp"

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    // FpssClient owns the streaming + reconnect surface.
    tdx::FpssClient fpss(creds, config);
    fpss.subscribe(tdx::Contract::stock("AAPL").quote());
    fpss.subscribe(tdx::Contract::option(
        "SPY", "20260116", "600", "C"
    ).quote());

    fpss.reconnect();
}
```
:::

## Complete Example

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    tdx.start_streaming(move |event: &FpssEvent| {
        match event {
            FpssEvent::Data(FpssData::Quote {
                contract, bid, ask, received_at_ns, ..
            }) => {
                println!("[QUOTE] {}: bid={bid:.2} ask={ask:.2} rx={received_at_ns}ns",
                    contract.symbol);
            }
            FpssEvent::Data(FpssData::Trade {
                contract, price, size, received_at_ns, ..
            }) => {
                println!("[TRADE] {}: price={price:.2} size={size} rx={received_at_ns}ns",
                    contract.symbol);
            }
            FpssEvent::Control(FpssControl::Reconnecting { reason, attempt, delay_ms }) => {
                eprintln!("Reconnecting (attempt {attempt}, {delay_ms}ms): {reason:?}");
            }
            FpssEvent::Control(FpssControl::ReconnectsExhausted { reason, attempts }) => {
                eprintln!("Recovery stopped after {attempts} attempts: {reason:?}");
                // Page an operator / rebuild the session out-of-band.
            }
            _ => {}
        }
    })?;

    tdx.subscribe(Contract::stock("AAPL").quote())?;
    tdx.subscribe(Contract::stock("AAPL").trade())?;
    tdx.subscribe(Contract::stock("MSFT").quote())?;

    // Block until interrupted
    std::thread::park();
    tdx.stop_streaming();
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient, Contract
import signal
import sys

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

# Graceful shutdown on Ctrl+C
def shutdown_handler(sig, frame):
    tdx.stop_streaming()
    sys.exit(0)

signal.signal(signal.SIGINT, shutdown_handler)

# Push-callback delivery via the `streaming(callback)` context
# manager. The `with` block pairs `stop_streaming()` + `await_drain()`
# on exit so the consumer thread has finished firing `on_event`
# before the scope returns.
def on_event(event):
    if event.kind == "quote":
        print(f"[QUOTE] {event.contract.symbol}: bid={event.bid} ask={event.ask} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "trade":
        print(f"[TRADE] {event.contract.symbol}: price={event.price} size={event.size} "
              f"rx={event.received_at_ns}ns")
    elif event.kind == "reconnecting":
        print(f"Reconnecting (attempt {event.attempt}): {event.reason_name}")
    elif event.kind == "reconnects_exhausted":
        print(f"Recovery stopped after {event.attempts} attempts: {event.reason_name}")

with tdx.streaming(on_event):
    tdx.subscribe(Contract.stock("AAPL").quote())
    tdx.subscribe(Contract.stock("AAPL").trade())
    tdx.subscribe(Contract.stock("MSFT").quote())
    import time
    time.sleep(60)
```
```cpp [C++]
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    auto client = tdx::UnifiedClient::connect(creds, config);

    // Typed control variants — one C struct per control event.
    // Dispatch on event.kind, read the matching event.<variant>
    // payload.
    client.set_callback([](const tdx::FpssEvent& event) {
        switch (event.kind) {
        case TDX_FPSS_QUOTE: {
            auto& q = event.quote;
            std::cout << "[QUOTE] " << q.contract.symbol
                      << " bid=" << q.bid << " ask=" << q.ask
                      << " rx=" << q.received_at_ns << "ns" << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            auto& t = event.trade;
            std::cout << "[TRADE] " << t.contract.symbol
                      << " price=" << t.price << " size=" << t.size << std::endl;
            break;
        }
        case TDX_FPSS_RECONNECTS_EXHAUSTED:
            std::cout << "Recovery stopped after "
                      << event.reconnects_exhausted.attempts
                      << " attempts, reason="
                      << event.reconnects_exhausted.reason << std::endl;
            break;
        default:
            break;
        }
    });

    // Subscribe via the unified contract-first API.
    client.subscribe(tdx::Contract::stock("AAPL").quote());
    client.subscribe(tdx::Contract::stock("AAPL").trade());
    client.subscribe(tdx::Contract::stock("MSFT").trade());

    // ... let the callback run ...
    client.stop_streaming();
}
```
:::
