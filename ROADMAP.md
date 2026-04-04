# ThetaDataDx Roadmap

## Endpoint Validation Status

Last validated: v5.2.1+ against live MDDS production (2026-04-04).
Data quality: 77/77 checks passed after OHLC + Greeks fixes.

### Stock Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 1 | `stock_list_symbols()` | 25,414 symbols | Verified | SPY/AAPL confirmed present |
| 2 | `stock_list_dates("TRADE", sym)` | 2,331 dates | Verified | v3: use "TRADE" not "EOD" |
| 3 | `stock_history_eod(sym, start, end)` | 63 ticks | Verified | OHLC valid, prices $500-700 |
| 4 | `stock_history_ohlc(sym, date, interval)` | 391 bars | Verified | 391/391 OHLC valid after #108 fix |
| 5 | `stock_history_trade(sym, date)` | 887,576 ticks | Verified | Use `_stream()` variant for large data. 0 bad prices. |
| 6 | `stock_history_quote(sym, date)` | 391 quotes | Verified | bid>0, ask>0, bid<ask, spread<$1 |
| 7 | `stock_history_trade_quote(sym, date)` | 519,448 ticks | Verified | AAPL via `_stream()`. 0 bad prices. |
| 8 | `stock_snapshot_ohlc(sym)` | 1 tick | Verified | OHLC valid |
| 9 | `stock_snapshot_trade(sym)` | 1 tick | Verified | price=$655.94, size=50 |
| 10 | `stock_snapshot_quote(sym)` | 1 tick | Verified | bid=$655.61, ask=$656.41 |
| 11 | `stock_snapshot_market_value(sym)` | 1 tick | Tier-limited | Returns zeros on STANDARD |
| 12 | `stock_at_time_trade(sym, date)` | OK | Verified | |
| 13 | `stock_at_time_quote(sym, date)` | OK | Verified | |

### Option Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 14 | `option_list_contracts(req, sym, date)` | 5,467 | Verified | Fixed in v5.2.1 (#97), v3 parser |
| 15 | `option_list_expirations(sym)` | 2,018 | Verified | 20260417 confirmed present |
| 16 | `option_list_strikes(sym, exp)` | 269 | Verified | |
| 17 | `option_list_dates(req, sym, exp, strike, right)` | OK | Verified | |
| 18 | `option_list_symbols()` | OK | Verified | |
| 19 | `option_history_eod(...)` | 22 ticks | Verified | OHLC valid on traded days |
| 20 | `option_history_ohlc(...)` | 391 bars | Verified | OHLC valid after #108 fix |
| 21 | `option_history_trade(...)` | 1 tick | Verified | price=$93.54, size>0 |
| 22 | `option_history_quote(...)` | 391 quotes | Verified | bid/ask non-negative |
| 23 | `option_history_trade_quote(...)` | 0 rows | Verified | Deep ITM 550C has no combined data (expected) |
| 24 | `option_history_open_interest(...)` | 1 tick | Verified | OI>=0 |
| 25 | `option_snapshot_ohlc(...)` | 1 tick | Verified | |
| 26 | `option_snapshot_trade(...)` | 1 tick | Verified | |
| 27 | `option_snapshot_quote(...)` | 1 tick | Verified | bid=$100.75, ask=$103.56 |
| 28 | `option_snapshot_open_interest(...)` | 1 tick | Verified | OI=52 |
| 29 | `option_snapshot_market_value(...)` | OK | Verified | |
| 30 | `option_snapshot_greeks_iv(...)` | OK | Tier-limited | IV=0 on STANDARD |
| 31 | `option_snapshot_greeks_all(...)` | Tier-gated | N/A | Requires Pro |
| 32 | `option_snapshot_greeks_first_order(...)` | OK | Tier-limited | Greeks zero on STANDARD |
| 33 | `option_snapshot_greeks_second_order(...)` | Tier-gated | N/A | Requires Pro |
| 34 | `option_snapshot_greeks_third_order(...)` | Tier-gated | N/A | Requires Pro |
| 35 | `option_history_greeks_eod(...)` | 23 rows | Verified | delta=0.97, iv=0.48 after #108 fix |
| 36 | `option_history_greeks_iv(...)` | 391 rows | Tier-limited | IV=0 on STANDARD |
| 37 | `option_history_greeks_all(...)` | Tier-gated | N/A | Requires Pro |
| 38 | `option_history_greeks_first_order(...)` | OK | Verified | |
| 39-44 | `option_history_greeks_second/third(...)` | Tier-gated | N/A | Requires Pro |
| 45 | `option_at_time_trade(...)` | 1 tick | Verified | price=$93.54 |
| 46 | `option_at_time_quote(...)` | 1 tick | Verified | bid=$99.66, ask=$101.76 |

### Index Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 47 | `index_list_symbols()` | 13,162 | Verified | SPX confirmed present |
| 48 | `index_list_dates("price", sym)` | OK | Verified | |
| 49 | `index_snapshot_ohlc(sym)` | Tier-gated | N/A | INDEX.STANDARD |
| 50 | `index_snapshot_price(sym)` | Tier-gated | N/A | INDEX.STANDARD |
| 51 | `index_snapshot_market_value(sym)` | Tier-gated | N/A | INDEX.VALUE |
| 52 | `index_history_eod(sym, start, end)` | 63 ticks | Verified | SPX $6,500-6,600 range |
| 53 | `index_history_ohlc(...)` | Tier-gated | N/A | INDEX.STANDARD |
| 54 | `index_history_price(...)` | Tier-gated | N/A | INDEX.STANDARD |
| 55 | `index_at_time_price(...)` | Tier-gated | N/A | INDEX.STANDARD |

### Calendar & Other

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 56 | `interest_rate_history_eod()` | Tier-gated | N/A | |
| 57 | `calendar_year("2025")` | 14 rows | Verified | Fixed in #110, v3 text parser |
| 58 | `calendar_on_date(date)` | 0 rows | OK | May only work during market hours |
| 59 | `calendar_open_today()` | 0 rows | OK | Same |

### FPSS Streaming

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| 60 | Quote subscription (stock) | Verified | Prices correct, bid/ask valid |
| 61 | Trade subscription (stock) | Verified | 8-field + 16-field formats handled |
| 62 | Quote subscription (option) | Pending | Not tested |
| 63 | Trade subscription (option) | Pending | Not tested |
| 64 | Open interest subscription | Pending | Not tested |
| 65 | Full trade firehose | Pending | Not tested |
| 66 | Full OI firehose | Pending | Not tested |
| 67 | Dev server replay | Verified | Binary Error frames + unknown codes handled (#85) |
| 68 | Reconnection | Verified | reconnect_streaming() tested |

### Wildcard Queries

| Scenario | Status | Notes |
|----------|--------|-------|
| `option_snapshot_ohlc("SPY", "*", "*", "C")` | Verified | 4,125 contracts returned. Server omits contract ID on snapshot OHLC. |
| Right parameter `"*"` | Fixed | Maps to `"both"` via `normalize_right()` |
| Right parameter `"C"` / `"P"` | Fixed | Maps to `"call"` / `"put"` for v3 server |

## Completed Work

- [x] **Data quality validation** -- 77/77 checks pass. OHLC prices fixed (#108), Greeks fixed (#108), calendar fixed (#110)
- [x] **v3 migration audit** -- all params use v3 names. `normalize_right()`, `normalize_interval()`, `symbol` in proto (#114)
- [x] **Lookup tables in tdbe** -- 78 exchanges, 149 trade conditions, 75 quote conditions, sequence tracking (#112)
- [x] **Error code mapping** -- 14 ThetaData HTTP codes mapped to human-readable names (#113)
- [x] **f64 price decoding** -- all SDKs return decoded prices by default (#95)
- [x] **Contract ID fields** -- wildcard queries return expiration/strike/right on every tick (#84)
- [x] **8-field trade format** -- dev server handled transparently (#86)
- [x] **Zero-JSON FFI** -- all repr(C) structs, serde_json removed (#82, #92)
- [x] **Right normalization** -- C->call, P->put, *->both. Go returns "C"/"P" string (#111)
- [x] **FPSS stability** -- binary Error frames skipped, unknown codes bounded retry (#85)

## Remaining Work

### High Priority

- [ ] **Option FPSS streaming** -- validate quote/trade subscriptions for option contracts
- [ ] **Pro-tier endpoint testing** -- test greeks_all, second/third order, trade_greeks when tier available
- [ ] **Cross-SDK parity** -- verify Python, Go, C++ all return identical data for the same query

### Medium Priority

- [x] **Large data streaming** -- SPY 887K trades, AAPL 519K trades via `_stream()` variant. 0 bad prices.
- [ ] **Auto-reconnect** -- optional auto-reconnect on FPSS disconnect (terminal has this, we don't)
- [ ] **Index streaming** -- verify subscribe_quotes for index contracts (SPX, VIX)

### Low Priority

- [ ] **Split/dividend endpoints** -- v3 docs say "Coming Soon"
- [ ] **request_type enum** -- expose typed constants to prevent user errors
- [ ] **Calendar edge cases** -- calendar_on_date/open_today return 0 rows
