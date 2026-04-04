---
title: Latency Measurement
description: Measure wire-to-application latency using received_at_ns, tdbe::latency::latency_ns(), and FpssFlushMode::Immediate.
---

# Latency Measurement

Every FPSS data event carries a `received_at_ns` field -- the wall-clock nanosecond timestamp captured the instant the frame is decoded in the I/O thread, before it enters the Disruptor ring buffer or reaches your callback.

Combined with the exchange's `ms_of_day` timestamp on each tick, this gives you wire-to-application latency per event.

## How it works

```mermaid
sequenceDiagram
    participant Exchange as Exchange (NYSE/NASDAQ)
    participant FPSS as ThetaData FPSS (NJ)
    participant SDK as ThetaDataDx SDK
    participant App as User Application

    Exchange->>FPSS: Market data feed
    Note over FPSS: FIT encode + delta compress
    FPSS->>SDK: TLS/TCP frame
    Note over SDK: received_at_ns captured
    SDK->>SDK: FIT decode + delta decompress
    SDK->>App: Callback (FpssEvent)
    Note over App: latency = received_at_ns - exchange_ns
```

The exchange stamps each quote/trade with `ms_of_day` (milliseconds since midnight ET). Your application stamps `received_at_ns` (nanoseconds since UNIX epoch). The difference is your total latency: exchange -> ThetaData -> network -> TLS -> decode -> your callback.

```mermaid
graph LR
    A["Exchange --> FPSS<br/>(~0ms)"] --> B["FPSS Processing<br/>(~0ms)"]
    B --> C["Network Transit<br/>(physics: distance/c)"]
    C --> D["SDK Decode<br/>(&lt; 1 us)"]
    D --> E["User Callback"]

    style C fill:#ff9999,color:#000
    style D fill:#99ff99,color:#000
```

The network transit segment (red) dominates total latency. The SDK decode time (green) is sub-microsecond and negligible.

::: danger Production only
Latency can only be measured meaningfully on the **production** FPSS server (`DirectConfig::production()`, port 20000) **during live market hours** (9:30 AM - 4:00 PM ET). The dev server (port 20200) replays historical data from a past trading day at maximum speed -- the exchange timestamps are from the past, so `received_at_ns` minus the event's original timestamp produces values that are months or years, not real latency. The dev server is for functional testing only, not latency benchmarking.
:::

## `tdbe::latency::latency_ns()`

The `tdbe` crate provides a DST-aware helper that converts the exchange `ms_of_day` + `date` into epoch nanoseconds and computes the delta:

```rust
pub fn latency_ns(exchange_ms_of_day: i32, event_date: i32, received_at_ns: u64) -> i64
```

**Parameters:**
- `exchange_ms_of_day`: from the tick (e.g., `34200000` = 9:30 AM ET)
- `event_date`: YYYYMMDD from the tick (e.g., `20260402`)
- `received_at_ns`: from `FpssData.received_at_ns`

**Returns:** Latency in nanoseconds. DST-aware (handles EST/EDT transitions automatically). Returns negative values if your clock is behind the exchange (clock skew).

## Rust

```rust
use thetadatadx::fpss::{FpssEvent, FpssData};
use tdbe::latency::latency_ns;

tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(FpssData::Quote {
            ms_of_day, date, received_at_ns, bid, ask, price_type, ..
        }) => {
            let lat_ns = latency_ns(*ms_of_day, *date, *received_at_ns);
            let lat_ms = lat_ns as f64 / 1_000_000.0;

            let b = tdbe::Price::new(*bid, *price_type).to_f64();
            let a = tdbe::Price::new(*ask, *price_type).to_f64();
            println!("SPY {:.2}/{:.2}  latency: {:.1}ms", b, a, lat_ms);
        }
        FpssEvent::Data(FpssData::Trade {
            ms_of_day, date, received_at_ns, price, size, price_type, ..
        }) => {
            let lat_ns = latency_ns(*ms_of_day, *date, *received_at_ns);
            let lat_us = lat_ns as f64 / 1_000.0;
            let p = tdbe::Price::new(*price, *price_type).to_f64();
            println!("TRADE {:.2} x{}  latency: {:.0}us", p, size, lat_us);
        }
        _ => {}
    }
})?;
```

## Python

```python
from thetadatadx import ThetaDataDx, Credentials, Config

tdx = ThetaDataDx(Credentials.from_file("creds.txt"), Config.production())
tdx.start_streaming()
tdx.subscribe_quotes("SPY")

while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        continue
    if event["kind"] == "quote":
        received_ns = event["received_at_ns"]
        # Convert exchange ms_of_day + date to epoch nanoseconds
        # (simplified -- for precise results use tdbe::latency_ns from Rust)
        import time
        now_ns = time.time_ns()
        approx_latency_ms = (now_ns - received_ns) / 1_000_000
        print(f"SPY {event['bid']:.2f}/{event['ask']:.2f}  "
              f"received_at_ns={received_ns}  "
              f"since_receive={approx_latency_ms:.1f}ms")
```

Note: in Python, `received_at_ns` is the Rust-side receive time. The delta between `received_at_ns` and `time.time_ns()` measures Rust-to-Python bridging overhead (typically <1ms). The true wire latency is best computed on the Rust side using `tdbe::latency::latency_ns()`.

## Go

```go
event, _ := fpss.NextEvent(5000)
if event != nil && event.Kind == thetadatadx.FpssQuoteEvent {
    q := event.Quote
    // received_at_ns is on the typed struct
    fmt.Printf("Quote rx=%d ns\n", q.ReceivedAtNs)

    // For precise latency, subtract exchange epoch ns from received_at_ns.
    // The exchange ms_of_day + date -> epoch conversion is best done
    // on the Rust side via tdbe::latency::latency_ns().
}
```

## C++

```cpp
auto event = fpss.next_event(5000);
if (event && event->kind == TDX_FPSS_QUOTE) {
    auto& q = event->quote;
    // received_at_ns is directly on the struct
    std::cout << "Quote rx=" << q.received_at_ns << "ns" << std::endl;
}
```

## Lowest Latency Configuration

For the absolute lowest latency:

1. **Use `FpssFlushMode::Immediate`** -- flushes after every frame write instead of batching to PING intervals:
   ```rust
   let mut config = DirectConfig::production();
   config.fpss_flush_mode = FpssFlushMode::Immediate;
   ```

2. **Keep the callback fast** -- the Disruptor callback runs on the consumer thread. Push to your own queue for heavy processing.

3. **Use the Rust SDK directly** -- Python, Go, and C++ add an mpsc channel hop between the Disruptor and `next_event()`.

## Network Physics: Minimum Achievable Latency

ThetaData's FPSS servers are located in New Jersey (NJ datacenter). The speed of light in fiber optic cable is approximately 200,000 km/s (about 2/3 of the vacuum speed of light, due to the refractive index of glass). This sets an absolute physical floor on latency that no software optimization can overcome.

The formula: `minimum_round_trip = distance_km / (300,000 * 0.67) * 2 * 1000` (in milliseconds).

| Your Location | Distance to NJ | Minimum Round-Trip | Typical Observed |
|---------------|---------------|-------------------|-----------------|
| AWS us-east-1 (Virginia) | ~350 km | ~3.5 ms | 2-5 ms |
| NJ/NYC datacenter | <50 km | <0.5 ms | <1 ms |
| Chicago | ~1,200 km | ~12 ms | 10-15 ms |
| Los Angeles | ~3,900 km | ~39 ms | 35-50 ms |
| London | ~5,600 km | ~56 ms | 55-70 ms |
| Frankfurt | ~6,200 km | ~62 ms | 60-80 ms |
| Tokyo | ~10,800 km | ~108 ms | 105-130 ms |
| Sydney | ~16,000 km | ~160 ms | 155-180 ms |

If you are seeing 60-80ms latency from Europe, that is not a bug -- it is the speed of light in fiber. No SDK, no protocol change, no configuration tweak can make photons travel faster.

The SDK's own overhead (`received_at_ns` capture, FIT decode, Disruptor dispatch, callback invocation) is sub-microsecond and entirely negligible compared to network physics.

For latency-sensitive applications:

1. **Colocate near NJ** -- AWS us-east-1 (N. Virginia) or any NJ/NYC-area datacenter gets sub-5ms
2. **`FpssFlushMode::Immediate`** reduces software batching latency by up to 100ms, but cannot beat physics
3. **Use the Rust SDK directly** -- eliminates the FFI channel hop present in Python/Go/C++ (adds <1ms)

## Latency Histogram Example (Rust)

```rust
use std::sync::{Arc, Mutex};
use thetadatadx::fpss::{FpssEvent, FpssData};
use tdbe::latency::latency_ns;

let buckets = Arc::new(Mutex::new(vec![0u64; 20])); // 0-10ms, 10-20ms, ...
let b = buckets.clone();

tdx.start_streaming(move |event: &FpssEvent| {
    if let FpssEvent::Data(FpssData::Quote { ms_of_day, date, received_at_ns, .. }) = event {
        let lat_ms = latency_ns(*ms_of_day, *date, *received_at_ns) / 1_000_000;
        let bucket = (lat_ms as usize / 10).min(19);
        b.lock().unwrap()[bucket] += 1;
    }
})?;

tdx.subscribe_quotes(&Contract::stock("SPY"))?;

// After collecting data:
std::thread::sleep(std::time::Duration::from_secs(60));
tdx.stop_streaming();

let h = buckets.lock().unwrap();
for (i, count) in h.iter().enumerate() {
    if *count > 0 {
        println!("{:>3}-{:>3}ms: {} events", i * 10, (i + 1) * 10, count);
    }
}
```

## `received_at_ns` on Every Data Event

Every `FpssData` variant includes this field:

| Variant | `received_at_ns` | Type |
|---------|-----------------|------|
| `Quote` | Present | `u64` |
| `Trade` | Present | `u64` |
| `OpenInterest` | Present | `u64` |
| `Ohlcvc` | Present | `u64` |

In Go: `event.Quote.ReceivedAtNs`, `event.Trade.ReceivedAtNs`, etc.
In C++: `event->quote.received_at_ns`, `event->trade.received_at_ns`, etc.
In Python: `event["received_at_ns"]` (integer).
