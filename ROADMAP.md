# ThetaDataDx Roadmap

This document is a **status of what works in production today** plus a **list of unfinished work**. Anything listed under a "Verified" stamp has been exercised end-to-end against the live ThetaData backend through the Rust SDK in this repo. Anything not yet shipped or not yet verified is in **Open Work**.

Versioning follows semver against the v8 line: `8.0.x` patches, `8.1.x` minor additions, `9.x.x` if and when a breaking change ships.

## Three Surfaces

The SDK exposes three independent ways to consume data:

| Surface | Purpose | Status |
|---------|---------|--------|
| **MDDS** | Request / response history + reference data | Verified |
| **FPSS** | Live streaming firehose (trades, quotes, OI) | Verified |
| **FLATFILES** | Server-pre-built whole-universe daily blobs delivered over the legacy MDDS auth path | Verified |

Each surface has a separate authentication flow and a separate transport. All three are reachable from the same `ThetaDataDx` client; consumers pick which surface they need on a per-call basis.

## MDDS — Endpoint Status

Last validator run: **2026-04-20** against live MDDS production.
Validator: `scripts/validate_cli.py` — full parameter-mode matrix, 134 cells.
Result: **127 PASS / 7 subscription-tier-blocked / 0 FAIL**. Every endpoint reachable on the test account passed on both the concrete-parameter path and every wildcard / bulk-chain variant covered by the matrix.

Run locally:
```bash
python3 scripts/validate_cli.py /path/to/creds.txt
```
Artifact: `artifacts/validator_cli.json`.

### Stock

| Endpoint | Tier | Status |
|----------|------|--------|
| `stock_list_symbols()` | Free | Verified |
| `stock_list_dates(req, sym)` | Free | Verified |
| `stock_history_eod(sym, start, end)` | Free | Verified |
| `stock_history_ohlc(sym, date, interval)` | Value | Verified |
| `stock_history_ohlc_range(sym, start, end, interval)` | Value | Verified |
| `stock_history_trade(sym, date)` | Standard | Verified |
| `stock_history_quote(sym, date)` | Value | Verified |
| `stock_history_trade_quote(sym, date)` | Standard | Verified |
| `stock_snapshot_ohlc(sym)` | Value | Verified |
| `stock_snapshot_trade(sym)` | Standard | Verified |
| `stock_snapshot_quote(sym)` | Value | Verified |
| `stock_snapshot_market_value(sym)` | Standard | Verified |
| `stock_at_time_trade(sym, date, time)` | Standard | Verified |
| `stock_at_time_quote(sym, date, time)` | Value | Verified |

### Option

| Endpoint | Tier | Status |
|----------|------|--------|
| `option_list_symbols()` | Free | Verified |
| `option_list_contracts(req, sym, date)` | Value | Verified |
| `option_list_expirations(sym)` | Free | Verified |
| `option_list_strikes(sym, exp)` | Free | Verified |
| `option_list_dates(req, sym, exp, strike, right)` | Free | Verified |
| `option_history_eod(...)` | Free | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_ohlc(...)` | Value | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_trade(...)` | Standard | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_quote(...)` | Value | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_trade_quote(...)` | Standard | Verified |
| `option_history_open_interest(...)` | Value | Verified |
| `option_snapshot_ohlc(...)` | Value | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_trade(...)` | Standard | Verified |
| `option_snapshot_quote(...)` | Value | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_open_interest(...)` | Value | Verified |
| `option_snapshot_market_value(...)` | Standard | Verified |
| `option_snapshot_greeks_implied_volatility(...)` | Standard | Verified |
| `option_snapshot_greeks_first_order(...)` | Standard | Verified |
| `option_snapshot_greeks_all(...)` | Professional | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_greeks_second_order(...)` | Professional | Verified |
| `option_snapshot_greeks_third_order(...)` | Professional | Verified |
| `option_history_greeks_eod(...)` | Standard | Verified |
| `option_history_greeks_implied_volatility(...)` | Standard | Verified |
| `option_history_greeks_first_order(...)` | Standard | Verified |
| `option_history_greeks_all(...)` | Professional | Verified |
| `option_history_greeks_second_order(...)` | Professional | Verified |
| `option_history_greeks_third_order(...)` | Professional | Verified |
| `option_history_trade_greeks_implied_volatility(...)` | Professional | Verified |
| `option_history_trade_greeks_all(...)` | Professional | Verified |
| `option_history_trade_greeks_first_order(...)` | Professional | Verified |
| `option_history_trade_greeks_second_order(...)` | Professional | Verified |
| `option_history_trade_greeks_third_order(...)` | Professional | Verified |
| `option_at_time_trade(...)` | Standard | Verified |
| `option_at_time_quote(...)` | Value | Verified |

### Index

| Endpoint | Tier | Status |
|----------|------|--------|
| `index_list_symbols()` | Free | Verified |
| `index_list_dates(req, sym)` | Free | Verified |
| `index_history_eod(sym, start, end)` | Free | Verified |
| `index_snapshot_ohlc(sym)` | Standard | Subscription-tier-blocked |
| `index_snapshot_price(sym)` | Standard | Subscription-tier-blocked |
| `index_snapshot_market_value(sym)` | Standard | Subscription-tier-blocked |
| `index_history_ohlc(...)` | Standard | Subscription-tier-blocked |
| `index_at_time_price(...)` | Value | Subscription-tier-blocked |
| `index_history_price(...)` | Value | Subscription-tier-blocked |

### Calendar, Interest Rate

| Endpoint | Tier | Status |
|----------|------|--------|
| `calendar_year(year)` | Value | Verified |
| `calendar_on_date(date)` | Value | Verified |
| `calendar_open_today()` | Free | Verified |
| `interest_rate_history_eod(sym, start, end)` | Value | Subscription-tier-blocked |

## FLATFILES — Surface Status

Live verified **2026-04-29 / 2026-04-30** against `nj-a.thetadata.us:12000`.
Reference output byte-matched against the vendor terminal jar's CSV for the same date.
Wire layer: TLS PacketStream (`[u32 size][u16 msg][i64 id][payload]`) with SPKI pinning, CREDENTIALS + VERSION login, FLAT_FILE request, chunked response, FLAT_FILE_END terminator.

| Endpoint | Tier | Status |
|----------|------|--------|
| `flatfile_option_open_interest(date, format)` | Standard | Verified (CSV byte-match + Parquet + JSONL row-count parity) |
| `flatfile_option_trade_quote(date, format)` | Standard | Verified |
| `flatfile_option_trade(date, format)` | Standard | Verified |
| `flatfile_option_quote(date, format)` | Standard | Verified |
| `flatfile_option_eod(date, format)` | Standard | Verified |
| `flatfile_stock_trade_quote(date, format)` | Stock-flatfile bundle | Subscription-tier-blocked |
| `flatfile_stock_trade(date, format)` | Stock-flatfile bundle | Subscription-tier-blocked |
| `flatfile_stock_quote(date, format)` | Stock-flatfile bundle | Subscription-tier-blocked |
| `flatfile_stock_eod(date, format)` | Stock-flatfile bundle | Subscription-tier-blocked |

Output formats: **CSV** (vendor-byte-equivalent), **Parquet** (zstd, columnar), **JSONL**. All three reproducible from `crates/thetadatadx/examples/flatfile_demo.rs`.

Server retention window: 7 calendar days. Older history: contact ThetaData sales for a deeper-history bundle.

## FPSS — Streaming Status

| Feature | Tier | Status |
|---------|------|--------|
| Stock quote subscription | Standard | Verified (prod + dev) |
| Stock trade subscription | Standard | Verified (prod + dev) |
| Option quote subscription | Standard | Verified |
| Option trade subscription | Standard | Verified |
| Open interest subscription | Pro | Verified |
| Full trade firehose (Option) | Pro | Verified |
| Full trade firehose (Stock) | Pro | Verified |
| Full OI firehose | Pro | Verified |
| Index price subscription | Indices subscription | Subscription-tier-blocked |
| Dev server replay | — | Verified |
| Reconnection (per-read deadline, Java-parity) | — | Verified |
| Mid-frame TCP pause tolerance | — | Verified |

## Open Work

### Cross-language parity for `utils`

The Rust SDK exposes `thetadatadx::utils::{conditions, exchange, sequences}` for post-processing tick records. The Python, TypeScript, Go, and C++ SDKs do **not** currently expose any of these helpers. Tracked in issue #424.

- [ ] **Python** — bind `thetadatadx.utils.{conditions, exchange, sequences}` via PyO3.
- [ ] **TypeScript** — bind via napi-rs under the same `utils.*` namespace.
- [ ] **Go** — flat helper functions `thetadatadx.UtilsConditionName(code)`, etc.
- [ ] **C++** — header at `sdks/cpp/include/thetadx_utils.h` with `extern "C"` bindings plus thin C++ wrappers.

### MDDS endpoint coverage on subscription-tier-blocked rows

The 7 SKIP rows in the MDDS validator are subscription-blocked on the current test account. They are exposed by the SDK, the wire calls compile, and they pass tier-rejected at the server. Verifying the success path requires either an account upgrade or a different validation account.

- [ ] Stock / Index / Interest-rate endpoints listed under **Subscription-tier-blocked** above, using a Value-tier or dedicated-Index account.

### FLATFILES on subscription-tier-blocked surfaces

- [ ] Stock flat files (`flatfile_stock_*`) — require the dedicated stock-flatfile bundle from sales. Wire path is identical to options; only the auth tier differs.
- [ ] Index + interest-rate flat files — same gate.

### Phase 2: relocate `utils` source from `tdbe` into `thetadatadx`

- [ ] Move `conditions.rs`, `exchange.rs`, `sequences.rs` from `crates/tdbe/src/` to `crates/thetadatadx/src/utils/`. Public path stays `thetadatadx::utils::*` so SDK consumers don't move. `tdbe` shrinks to its actual scope (codec primitives, format spec, tick types, Greeks math) and bumps to a major version.

### Cross-SDK parity validation

- [ ] Run the validator matrix through the Python, TypeScript, Go, and C++ SDKs and compare row-for-row against the Rust artifact. Locks in the contract that all four bindings return identical data for every endpoint.

### Upstream features

- [ ] Split / dividend endpoints — listed by ThetaData as "Coming Soon" upstream. SDK-side work is gated on the wire surface landing.
