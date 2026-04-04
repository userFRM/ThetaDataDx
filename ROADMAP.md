# ThetaDataDx Roadmap

## Endpoint Validation Status

Last validated: v5.2.1 against live MDDS production (2026-04-04).

### Stock Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 1 | `stock_list_symbols()` | OK | Pending | |
| 2 | `stock_list_dates("trade", sym)` | OK | Pending | |
| 3 | `stock_history_eod(sym, start, end)` | OK | Pending | |
| 4 | `stock_history_ohlc(sym, date, interval)` | OK | Pending | |
| 5 | `stock_history_trade(sym, date)` | Timeout | Pending | SPY full-day is millions of rows |
| 6 | `stock_history_quote(sym, date)` | OK | Pending | |
| 7 | `stock_history_trade_quote(sym, date)` | Timeout | Pending | SPY full-day is millions of rows |
| 8 | `stock_snapshot_ohlc(sym)` | OK | Pending | |
| 9 | `stock_snapshot_trade(sym)` | OK | Pending | |
| 10 | `stock_snapshot_quote(sym)` | OK | Pending | |
| 11 | `stock_snapshot_market_value(sym)` | OK | Pending | |
| 12 | `stock_at_time_trade(sym, date)` | OK | Pending | |
| 13 | `stock_at_time_quote(sym, date)` | OK | Pending | |

### Option Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 14 | `option_list_contracts(req, sym, date)` | OK | Pending | Fixed in v5.2.1 (#97) |
| 15 | `option_list_expirations(sym)` | OK | Pending | |
| 16 | `option_list_strikes(sym, exp)` | OK | Pending | |
| 17 | `option_list_dates(req, sym, exp, strike, right)` | OK | Pending | |
| 18 | `option_history_eod(...)` | OK | Pending | |
| 19 | `option_history_ohlc(...)` | OK | Pending | |
| 20 | `option_history_trade(...)` | OK | Pending | |
| 21 | `option_history_quote(...)` | OK | Pending | |
| 22 | `option_history_trade_quote(...)` | 0 rows | Pending | Deep ITM, may have no data |
| 23 | `option_history_open_interest(...)` | OK | Pending | |
| 24 | `option_snapshot_ohlc(...)` | OK | Pending | |
| 25 | `option_snapshot_trade(...)` | OK | Pending | |
| 26 | `option_snapshot_quote(...)` | OK | Pending | |
| 27 | `option_snapshot_open_interest(...)` | OK | Pending | |
| 28 | `option_snapshot_market_value(...)` | OK | Pending | |
| 29 | `option_snapshot_greeks_iv(...)` | OK | Pending | |
| 30 | `option_snapshot_greeks_all(...)` | Tier-gated | N/A | Requires Pro |
| 31 | `option_snapshot_greeks_first_order(...)` | OK | Pending | |
| 32 | `option_snapshot_greeks_second_order(...)` | Tier-gated | N/A | Requires Pro |
| 33 | `option_snapshot_greeks_third_order(...)` | Tier-gated | N/A | Requires Pro |
| 34 | `option_history_greeks_eod(...)` | OK | Pending | |
| 35 | `option_history_greeks_iv(...)` | OK | Pending | |
| 36 | `option_history_greeks_all(...)` | Tier-gated | N/A | Requires Pro |
| 37 | `option_history_greeks_first_order(...)` | OK | Pending | |
| 38 | `option_history_greeks_second_order(...)` | Tier-gated | N/A | Requires Pro |
| 39 | `option_history_greeks_third_order(...)` | Tier-gated | N/A | Requires Pro |
| 40 | `option_history_trade_greeks_iv(...)` | Tier-gated | N/A | Requires Pro |
| 41 | `option_history_trade_greeks_all(...)` | Tier-gated | N/A | Requires Pro |
| 42 | `option_history_trade_greeks_first_order(...)` | Tier-gated | N/A | Requires Pro |
| 43 | `option_history_trade_greeks_second_order(...)` | Tier-gated | N/A | Requires Pro |
| 44 | `option_history_trade_greeks_third_order(...)` | Tier-gated | N/A | Requires Pro |
| 45 | `option_at_time_trade(...)` | OK | Pending | |
| 46 | `option_at_time_quote(...)` | OK | Pending | |

### Index Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 47 | `index_list_symbols()` | OK | Pending | |
| 48 | `index_list_dates("price", sym)` | OK | Pending | |
| 49 | `index_snapshot_ohlc(sym)` | Tier-gated | N/A | INDEX.STANDARD |
| 50 | `index_snapshot_price(sym)` | Tier-gated | N/A | INDEX.STANDARD |
| 51 | `index_snapshot_market_value(sym)` | Tier-gated | N/A | INDEX.VALUE |
| 52 | `index_history_eod(sym, start, end)` | OK | Pending | INDEX.FREE |
| 53 | `index_history_ohlc(sym, start, end, interval)` | Tier-gated | N/A | INDEX.STANDARD |
| 54 | `index_history_price(sym, start, end)` | Tier-gated | N/A | INDEX.STANDARD |
| 55 | `index_at_time_price(sym, date)` | Tier-gated | N/A | INDEX.STANDARD |

### Other Endpoints

| # | Endpoint | Returns | Data Quality | Notes |
|---|----------|---------|-------------|-------|
| 56 | `interest_rate_history_eod()` | Tier-gated | N/A | |
| 57 | `calendar_market_hours(start, end)` | OK | Pending | |

### FPSS Streaming

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| 58 | Quote subscription (stock) | OK | Verified live |
| 59 | Trade subscription (stock) | OK | 8-field + 16-field formats handled |
| 60 | Quote subscription (option) | Pending | Not tested |
| 61 | Trade subscription (option) | Pending | Not tested |
| 62 | Open interest subscription | Pending | Not tested |
| 63 | Full trade firehose | Pending | Not tested |
| 64 | Full OI firehose | Pending | Not tested |
| 65 | Dev server replay | OK | Binary Error frames + unknown codes handled |
| 66 | Reconnection | OK | reconnect_streaming() tested |

## Wildcard Query Validation

| Scenario | Status | Notes |
|----------|--------|-------|
| `option_snapshot_*("SPY", "*", "*", "*")` | Pending | Contract ID fields populated? |
| `option_history_*("SPY", "*", "*", "*", date)` | Pending | Contract ID fields populated? |

## Upcoming Work

### High Priority

- [ ] **Data quality validation** -- verify every working endpoint returns correct, logical data (prices in range, fields populated, dates match)
- [ ] **v3 migration audit** -- check all endpoints use v3 field names (symbol not root, etc.) per [migration guide](https://docs.thetadata.us/Articles/Getting-Started/v2-migration-guide.html)
- [ ] **Wildcard contract ID validation** -- confirm expiration/strike/right fields are populated on bulk queries

### Medium Priority

- [ ] **Lookup tables in tdbe** -- trade conditions (149), quote conditions, exchange codes (78) as const data files
- [ ] **request_type constants** -- expose `REQUEST_TYPE_TRADE` / `REQUEST_TYPE_QUOTE` to prevent user errors like #97
- [ ] **Option FPSS streaming** -- validate quote/trade subscriptions for option contracts
- [ ] **Pro-tier endpoint testing** -- test greeks_all, second/third order, trade_greeks when tier available

### Low Priority

- [ ] **Large data streaming** -- test stock_history_trade with chunked/streamed processing
- [ ] **Interval normalization audit** -- verify all interval formats work (1s, 5s, 1m, 5m, 15m, 30m, 1h)
- [ ] **Cross-SDK parity** -- verify Python, Go, C++ all return identical data for the same query
