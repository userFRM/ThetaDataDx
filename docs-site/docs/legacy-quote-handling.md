---
title: Legacy quote handling (issue #571)
---

# Legacy 6-field NBBO quote rows on 2022-era options

ThetaData's MDDS server emits some 2022-era option NBBO rows in the
pre-extension 6-field layout
(`ms_of_day, bid_size, bid, ask_size, ask, date`) rather than the
current 11-field layout that carries exchange codes, condition flags,
and a `price_type` discriminator. The local ThetaTerminal's Java
`QuoteTick` constructor validates the row length against `11` and
throws `IllegalArgumentException` on the 6-field shape. The exception
bubbles through the gRPC handler and terminates the HTTP/2 stream
with no error frame; the SDK observes
`TransportErrorKind::ConnectionClosed` mid-response and the call
fails with no recovery path.

The bug surfaces on the four quote-bearing endpoints:

* `option_history_quote`
* `option_history_trade_quote`
* `option_history_greeks_implied_volatility`
* `option_history_greeks_first_order`

Sibling endpoints (`option_history_trade`,
`option_history_open_interest`) work for the same dates because they
do not touch the NBBO storage rows.

This page documents the three escape hatches the SDK ships in
v10.x, in order of operational simplicity.

## 1. REST fallback (zero local-Terminal modification)

The SDK ships a REST transport against the local Terminal's
`/v3/...` HTTP paths. HTTP/1.1 is per-request rather than h2 stream
multiplexing, so the same per-row Java exception cannot cascade
across multiple responses. The REST path additionally serves the
legacy 6-field rows verbatim -- the SDK's CSV decoder is lenient on
the absent `bid_exchange` / `bid_condition` / `ask_exchange` /
`ask_condition` columns and defaults them to `0`.

### Usage

#### Rust

```rust
use thetadatadx::{
    Credentials, DirectConfig, FallbackPolicy, ThetaDataDxClient,
    DEFAULT_REST_BASE_URL,
};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let cfg = DirectConfig::production().with_rest_fallback(
        FallbackPolicy::RestAlwaysForDateRange {
            base_url: DEFAULT_REST_BASE_URL.to_string(),
            before: 20_230_101, // YYYYMMDD cutoff
        },
    );
    let tdx = ThetaDataDxClient::connect(
        &Credentials::from_file("creds.txt")?,
        cfg,
    ).await?;

    // Pre-2023 -- routes over REST automatically. No h2 cascade.
    let ticks = tdx.option_history_quote_with_fallback(
        "QQQ", "20220414", "20220414", /* end_date */ None,
        /* strike */ Some("330"),
        /* right  */ Some("call"),
        /* interval */ Some("1m"),
    ).await?;

    // 2024+ -- flows through gRPC as normal.
    let ticks = tdx.option_history_quote_with_fallback(
        "QQQ", "20240605", "20240604", None,
        Some("440"), Some("call"), Some("1m"),
    ).await?;

    Ok(())
}
```

#### Python

```python
import thetadatadx as m

creds = m.Credentials.from_file("creds.txt")
cfg = m.Config.production()
cfg.with_rest_fallback(
    m.FallbackPolicy.rest_always_for_date_range(
        m.DEFAULT_REST_BASE_URL, before=20230101
    )
)
tdx = m.ThetaDataDxClient(creds, cfg)

# Pre-2023 -- routes over REST.
ticks = tdx.option_history_quote_with_fallback(
    symbol="QQQ",
    expiration="20220415",
    start_date="20220414",
    strike="330",
    right="call",
    interval="1m",
)

# 2024+ -- flows through gRPC as normal. Same call signature.
ticks = tdx.option_history_quote_with_fallback(
    symbol="QQQ",
    expiration="20240620",
    start_date="20240605",
    strike="440",
    right="call",
    interval="1m",
)
```

#### TypeScript / C++

The `FallbackPolicy` class + `_with_fallback` methods are wired on the
Rust core + Python binding in v10.x. TypeScript and C++ ports are
tracked as a follow-up; until then, TypeScript and C++ consumers
should call the standalone `RestClient` directly when targeting
pre-2023 dates (the underlying HTTP path is the same).

### Policy variants

| Variant | Behaviour |
|---|---|
| `Disabled` (default) | Always gRPC. No fallback. |
| `RestOnH2Disconnect { base_url }` | Try gRPC; on `ConnectionClosed` re-issue over REST. |
| `RestAlwaysForDateRange { base_url, before }` | `start_date < before` → REST immediately; else gRPC. |
| `RestAlways { base_url }` | Always REST. |

`RestAlwaysForDateRange` is the recommended setting when the caller
knows the symbol's storage rows are split across the schema
rollover. It saves the failed-gRPC round-trip cost on every legacy
call while keeping the gRPC fast path for current-shape rows.

### REST endpoint coverage in v10.x

The four endpoints from issue #571's failure matrix, each exposed on
`ThetaDataDxClient` as a `*_with_fallback` shim that consults the
[`FallbackPolicy`](#policy-variants) before dispatching:

| gRPC endpoint | High-level shim | REST builder |
|---|---|---|
| `option_history_quote` | `option_history_quote_with_fallback` | `RestClient::option_history_quote` |
| `option_history_trade_quote` | `option_history_trade_quote_with_fallback` | `RestClient::option_history_trade_quote` |
| `option_history_greeks_implied_volatility` | `option_history_greeks_implied_volatility_with_fallback` | `RestClient::option_history_greeks_implied_volatility` |
| `option_history_greeks_first_order` | `option_history_greeks_first_order_with_fallback` | `RestClient::option_history_greeks_first_order` |

The greeks shims (added in #577) follow the same dispatch semantics
as the quote pair: pre-route to REST when `pre_routes_to_rest`
fires, otherwise try gRPC and on `ConnectionClosed` re-issue over
REST. The greeks endpoints reach back to NBBO storage rows for the
underlying snapshot at each interval, so the same #571 cascade
applies; the shim removes the manual `RestClient::...` call users
otherwise had to write.

Other historical endpoints can be added with the same shape; open
an issue if you need one extended.

### Direct REST usage

If you want to bypass the gRPC builder entirely, [`crate::rest::RestClient`]
is a standalone HTTP transport:

```rust
use thetadatadx::rest::RestClient;

let rest = RestClient::new("http://127.0.0.1:25503")?;
let ticks = rest
    .option_history_quote("QQQ", "20220414", "20220414")
    .strike("330")
    .right("call")
    .interval("1m")
    .execute()
    .await?;
```

The returned `Vec<QuoteTick>` is the same shape as the gRPC path.

## 2. Patched Terminal (server-side fix)

For workloads that prefer to keep using gRPC, the
`local-terminal-patcher` binary in `tools/local-terminal-patcher/`
rewrites the inner library jar so the Java `QuoteTick` constructor
upcasts 6-field rows to the 11-field shape rather than throwing.
Once the patched jar is in place, the gRPC h2 cascade does not
trigger and 2022-era queries flow through the standard transport.

```sh
cargo run -p local-terminal-patcher -- \
    --terminal-dir ~/ThetaData/ThetaTerminal
```

The tool autodetects the inner jar (`<dir>/lib/<latest>.jar`),
verifies it carries the known-broken bytecode signature
(`bipush 11 / if_icmpeq`), recompiles the patches via system
`javac` (JDK 11+), and emits `<latest>-patched.jar` beside the
original. The CLI prints the post-patch launcher recipe and the
`FallbackPolicy` snippet to drop into the SDK config.

The Terminal's auto-updater will overwrite the inner jar on next
launch -- pin the patched jar in place by `chmod -w`ing the lib/
directory, or run the Terminal with the auto-update flag disabled.

### Patch contents

* `QuoteTick.java` -- adds `normalizeData()` that upcasts 6-field
  rows (`ms_of_day, bid_size, bid, ask_size, ask, date`) to the
  current 11-field layout by zero-filling the absent exchange /
  condition / price_type columns. Genuine corruption (length other
  than 6 or 11) throws a diagnostic exception with the actual array
  contents so the storage team can identify the upstream record.

* `OhlcTick.java` -- cosmetic fix on the OHLC constructor's error
  message ("must be 10" → "expected length 9", matching the actual
  check). No functional change on the parse path.

Full root-cause analysis lives in
`tools/local-terminal-patcher/patches/PATCH_NOTES.md`.

## 3. Flat-file workaround (no SDK change needed)

The legacy flat-file API serves 2022-era data through the
`flatfile_option_quote` / `flatfile_option_trade_quote` paths.
Trades off bandwidth efficiency (the flat-file API streams the full
daily OPRA dump per call) for transport correctness. Useful as a
last-resort recovery when neither the patched Terminal nor the REST
fallback is available.

```rust
let path = tdx.flatfile_option_quote("20220414", "/tmp/").await?;
```

See [Flat files](flatfiles/index.md) for the full API.

## Channel-layer recovery (issue #577)

When the upstream cascade fires on a streaming RPC, the SDK observes
`Error::Transport { kind: ConnectionClosed, .. }` mid-response. Two
behaviours land in v10.x to recover automatically:

1. **Per-channel death tracking.** Each pooled `Channel` carries an
   `AtomicBool` death flag. The flag flips as soon as a poll on any
   `ServerStreaming` adapter surfaces `ChannelError::ConnectionClosed`
   (or any open-phase `ready()` / `send_request()` / `send_data()`
   call fails with the same classification). The pool's `next()`
   picker treats dead channels as last-resort: it scans for the
   least-loaded LIVE channel and only routes to a dead member when
   every channel in the pool is dead. The picker never blocks.

2. **Classifier-level retry.** The streaming and unary retry loops
   in `mdds::macros` now treat `Error::Transport { kind:
   ConnectionClosed, .. }` as `Transient`. Each retry iteration
   reissues the RPC by calling `self.channel()` afresh -- which
   under the dead-channel routing picks a live member. The retry
   policy's `max_attempts` budget bounds the recovery loop; if every
   channel in the pool dies the cascade surfaces after the budget is
   exhausted, matching previous user-visible behaviour for the worst
   case while fixing the common case.

The two combined mean a single 6-field-row cascade no longer pinballs
the same dead h2 channel on every subsequent dispatch. A pool of 4
channels with one dead member behaves like a pool of 3.

The `FallbackPolicy` integration in
`*_with_fallback` is still the recommended top-level recovery for
known legacy date ranges -- it skips the failed gRPC round-trip on
every pre-cutoff call. Channel-layer recovery is the safety net for
calls that *do* reach gRPC and observe the cascade mid-response.

## Decoder behaviour (gRPC path)

The SDK's gRPC `parse_quote_ticks` decoder is also lenient on absent
columns: `bid_exchange`, `bid_condition`, `ask_exchange`,
`ask_condition` are declared optional in `tick_schema.toml`, so
when a future upstream patch lands and starts serving the upcast
rows over gRPC the decoder picks them up without any further
change. The regression tests
`quote_tick_decodes_legacy_six_field_shape_with_zero_fill` and
`quote_tick_decodes_current_eleven_field_shape_unchanged` in
`crates/thetadatadx/src/mdds/decode/tests.rs` pin both paths.

## Verification

Live-verify the REST fallback against your local Terminal:

```sh
THETADATA_EMAIL=... THETADATA_PASSWORD=... \
THETADX_LIVE_PATCHED_TERMINAL=1 \
cargo test -p thetadatadx --tests rest_live -- --ignored
```

The live-gated integration test in
`crates/thetadatadx/tests/` (when added) drives the patched Terminal
through a 2022 reproducer and asserts a non-empty response.

## References

* GitHub issue: [#571](https://github.com/userFRM/ThetaDataDx/issues/571)
* Patch sources: `tools/local-terminal-patcher/patches/`
* SDK module: [`crate::rest`](https://docs.rs/thetadatadx/latest/thetadatadx/rest/index.html)
* Fallback policy: [`crate::FallbackPolicy`](https://docs.rs/thetadatadx/latest/thetadatadx/enum.FallbackPolicy.html)
