# Latency Measurement

Every FPSS streaming event carries a `received_at_ns` field -- the wall-clock nanosecond timestamp captured the instant the frame is decoded in the I/O thread, before it reaches your callback.

Combined with the exchange's `ms_of_day` timestamp on each tick, this gives you wire-to-application latency per event.

## How it works

```
Exchange (NJ) ──── ThetaData FPSS server ──── TLS/TCP ──── Your application
    |                                                              |
    ms_of_day                                              received_at_ns
    (exchange clock)                                       (your clock)
    
    latency = received_at_ns - exchange_timestamp_ns
```

The exchange stamps each quote/trade with `ms_of_day` (milliseconds since midnight ET). Your application stamps `received_at_ns` (nanoseconds since UNIX epoch). The difference is your total latency: exchange -> ThetaData -> network -> TLS -> decode -> your callback.

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

## Latency histogram example (Rust)

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

## API reference

### `FpssData` fields

Every `FpssData` variant (Quote, Trade, OpenInterest, Ohlcvc) includes:

| Field | Type | Description |
|-------|------|-------------|
| `received_at_ns` | `u64` | Wall-clock nanoseconds since UNIX epoch, captured at frame decode |

### `tdbe::latency::latency_ns`

```rust
pub fn latency_ns(exchange_ms_of_day: i32, event_date: i32, received_at_ns: u64) -> i64
```

Computes wire-to-application latency in nanoseconds. DST-aware (handles EST/EDT transitions). Returns negative values if your clock is behind the exchange (clock skew).

**Parameters:**
- `exchange_ms_of_day`: from the tick (e.g., `34200000` = 9:30 AM ET)
- `event_date`: YYYYMMDD from the tick (e.g., `20260402`)
- `received_at_ns`: from `FpssData.received_at_ns`
