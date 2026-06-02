# thetadatadx

Native Rust SDK for ThetaData market data. Speaks ThetaData's wire
protocols directly — historical gRPC, streaming TCP, and the native
flat-file distribution for bulk pulls — without a JVM or a local
proxy. One async client object speaks every transport directly.

[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg?logo=rust)](https://crates.io/crates/thetadatadx)
[![docs.rs](https://img.shields.io/docsrs/thetadatadx?logo=docsdotrs)](https://docs.rs/thetadatadx)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE)

> **Requires a valid [ThetaData](https://thetadata.us) subscription.**
> The SDK authenticates against the Nexus API with your account
> credentials.

## Install

```toml
[dependencies]
thetadatadx = "12"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Opt-in DataFrame ergonomics:

```toml
thetadatadx = { version = "12", features = ["polars"] }   # or "arrow"
```

## Historical

```rust
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    // EOD Greeks for a SPY option chain across Q1.
    let chain = tdx
        .option_history_greeks_eod("SPY", "20260619", "20240101", "20240331")
        .await?;

    for t in chain.iter().take(5) {
        println!(
            "{} K={:>6.2} {} delta={:+.4} gamma={:+.4} theta={:+.4} IV={:.4}",
            t.date, t.strike, t.right, t.delta, t.gamma, t.theta, t.implied_vol,
        );
    }
    Ok(())
}
```

61 typed endpoints across stock, option, index, calendar, and
interest-rate surfaces. Each builder accepts `.await` for a
buffered `Vec<Tick>` or `.stream(handler)` for chunk-by-chunk
delivery on multi-day backfills.

## Streaming

Two equivalent shapes. Pick whichever fits the call site.

### Unified — `ThetaDataDxClient`

One auth, one connection. Historical works immediately; streaming
opens on the first `start_streaming(callback)` call. Subscribe after
the callback is registered.

```rust
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::prelude::*;

tdx.start_streaming(|event: &FpssEvent| {
    if let FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) = event {
        println!("{} @ {price} x {size}", contract.symbol);
    }
})?;

tdx.subscribe(Contract::stock("AAPL").quote())?;
tdx.subscribe(Contract::option("SPY", "20260620", "550", "C")?.trade())?;
```

### Standalone — `FpssClient`

Streaming-only workloads. No MDDS / Nexus session, no historical
surface. Drive the iterator directly:

```rust
use thetadatadx::auth::Credentials;
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::{FpssClient, FpssEvent};
use thetadatadx::fpss::protocol::Contract;

let creds = Credentials::from_file("creds.txt")?;
let hosts = DirectConfig::production().fpss.hosts;
let client = FpssClient::builder(&creds, &hosts)
    .ring_size(8192)
    .build()?;

client.subscribe(Contract::stock("AAPL").quote())?;

for event in &client {
    match event? {
        FpssEvent::Data(data)       => { /* … */ }
        FpssEvent::Control(control) => { /* … */ }
    }
}
```

Drain primitives on `FpssClient`:

- `next_event()` — block until the next event or terminal shutdown.
- `try_next_event()` — non-blocking single pop.
- `poll_batch(|event| …)` — non-blocking batch drain.
- `for_each(|event| …)` — blocking loop until shutdown.
- `for event in &client { … }` — iterator over `Result<FpssEvent, FpssError>`.

Auto-reconnect with subscription replay is on by default; tune via
`FpssClientBuilder` or `DirectConfig::reconnect`.

## Auth

Three ways to supply credentials:

```rust
let creds = Credentials::from_file("creds.txt")?;                    // two-line file
let creds = Credentials::new("user@example.com", "password");        // inline
// or set THETADATA_EMAIL / THETADATA_PASSWORD and read via env
```

## Errors

Every public method returns `Result<_, thetadatadx::Error>`. The FPSS
streaming surface adds a typed `FpssError` enum for the polling and
subscribe paths; the umbrella `Error` is reachable via
`From<FpssError> for Error` for callers that prefer a single error
type.

## Performance

- Zero-copy decode where the wire format allows (no Vec churn between
  the codec and the typed tick row).
- Native HTTP/2 transport with no intermediary layer.
- Single-producer single-consumer ring on the streaming hot path; the
  TLS reader never blocks on user code.
- Optional `polars` / `arrow` features for zero-copy DataFrame
  conversion against Arrow C Data Interface.

## More

- [Repository, issues, contributing](https://github.com/userFRM/ThetaDataDx)
- [API reference](https://docs.rs/thetadatadx)
- [Documentation site](https://userfrm.github.io/ThetaDataDx/) — guides,
  per-endpoint pages, streaming deep-dive.
- Python (`pip install thetadatadx`), Node.js (`npm install
  thetadatadx`), and C++ bindings share the same wire-format and
  reconnect behaviour.

## License

Apache-2.0.
