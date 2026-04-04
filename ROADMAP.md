# ThetaDataDx Roadmap

## Endpoint Status

Last validated: 2026-04-04 against live MDDS production.

### Stock

| Endpoint | Tier | Status |
|----------|------|--------|
| `stock_list_symbols()` | Standard | Verified |
| `stock_list_dates(req, sym)` | Standard | Verified |
| `stock_history_eod(sym, start, end)` | Standard | Verified |
| `stock_history_ohlc(sym, date, interval)` | Standard | Verified |
| `stock_history_trade(sym, date)` | Standard | Verified |
| `stock_history_quote(sym, date)` | Standard | Verified |
| `stock_history_trade_quote(sym, date)` | Standard | Verified |
| `stock_snapshot_ohlc(sym)` | Standard | Verified |
| `stock_snapshot_trade(sym)` | Standard | Verified |
| `stock_snapshot_quote(sym)` | Standard | Verified |
| `stock_snapshot_market_value(sym)` | Value | Not tested |
| `stock_at_time_trade(sym, date)` | Standard | Verified |
| `stock_at_time_quote(sym, date)` | Standard | Verified |

### Option

| Endpoint | Tier | Status |
|----------|------|--------|
| `option_list_symbols()` | Standard | Verified |
| `option_list_contracts(req, sym, date)` | Standard | Verified |
| `option_list_expirations(sym)` | Standard | Verified |
| `option_list_strikes(sym, exp)` | Standard | Verified |
| `option_list_dates(req, sym, exp, strike, right)` | Standard | Verified |
| `option_history_eod(...)` | Standard | Verified |
| `option_history_ohlc(...)` | Standard | Verified |
| `option_history_trade(...)` | Standard | Verified |
| `option_history_quote(...)` | Standard | Verified |
| `option_history_trade_quote(...)` | Standard | Verified |
| `option_history_open_interest(...)` | Standard | Verified |
| `option_snapshot_ohlc(...)` | Standard | Verified |
| `option_snapshot_trade(...)` | Standard | Verified |
| `option_snapshot_quote(...)` | Standard | Verified |
| `option_snapshot_open_interest(...)` | Standard | Verified |
| `option_snapshot_market_value(...)` | Standard | Verified |
| `option_snapshot_greeks_iv(...)` | Pro | Not tested |
| `option_snapshot_greeks_all(...)` | Pro | Not tested |
| `option_snapshot_greeks_first_order(...)` | Pro | Not tested |
| `option_snapshot_greeks_second_order(...)` | Pro | Not tested |
| `option_snapshot_greeks_third_order(...)` | Pro | Not tested |
| `option_history_greeks_eod(...)` | Pro | Not tested |
| `option_history_greeks_iv(...)` | Pro | Not tested |
| `option_history_greeks_all(...)` | Pro | Not tested |
| `option_history_greeks_first_order(...)` | Pro | Not tested |
| `option_history_greeks_second_order(...)` | Pro | Not tested |
| `option_history_greeks_third_order(...)` | Pro | Not tested |
| `option_history_trade_greeks_iv(...)` | Pro | Not tested |
| `option_history_trade_greeks_all(...)` | Pro | Not tested |
| `option_history_trade_greeks_first_order(...)` | Pro | Not tested |
| `option_history_trade_greeks_second_order(...)` | Pro | Not tested |
| `option_history_trade_greeks_third_order(...)` | Pro | Not tested |
| `option_at_time_trade(...)` | Standard | Verified |
| `option_at_time_quote(...)` | Standard | Verified |

### Index

| Endpoint | Tier | Status |
|----------|------|--------|
| `index_list_symbols()` | Free | Verified |
| `index_list_dates(req, sym)` | Free | Verified |
| `index_snapshot_ohlc(sym)` | Standard | Not tested |
| `index_snapshot_price(sym)` | Standard | Not tested |
| `index_snapshot_market_value(sym)` | Value | Not tested |
| `index_history_eod(sym, start, end)` | Free | Verified |
| `index_history_ohlc(...)` | Standard | Not tested |
| `index_history_price(...)` | Standard | Not tested |
| `index_at_time_price(...)` | Standard | Not tested |

### Calendar & Other

| Endpoint | Tier | Status |
|----------|------|--------|
| `interest_rate_history_eod()` | Standard | Not tested |
| `calendar_year(year)` | Free | Verified |
| `calendar_on_date(date)` | Free | Verified |
| `calendar_open_today()` | Free | Verified |

### FPSS Streaming

Dev server replays April 28, 2025. Stock data only -- option FPSS does not replay on dev.

| Feature | Tier | Status |
|---------|------|--------|
| Stock quote subscription | Standard | Verified |
| Stock trade subscription | Standard | Verified |
| Option quote subscription | Standard | Verified (subscribed, no data on dev replay) |
| Option trade subscription | Standard | Verified (subscribed, no data on dev replay) |
| Open interest subscription | Pro | InvalidPerms on Standard |
| Full trade firehose | Pro | InvalidPerms on Standard |
| Full OI firehose | Pro | InvalidPerms on Standard |
| Index price subscription | Free | Not tested |
| Dev server replay | -- | Verified |
| Reconnection | -- | Verified |

## Remaining Work

### Needs Pro Tier

- [ ] Test FPSS OI, full trade firehose, full OI firehose (InvalidPerms on Standard)
- [ ] Test all greeks/trade_greeks snapshot + history endpoints
- [ ] Test stock/index market_value endpoints (Value tier)

### Remaining

- [x] Test FPSS stock streaming (quotes + trades verified on dev)
- [x] Test FPSS option streaming (subscriptions accepted, no data on dev replay)
- [x] Large data streaming (887K SPY trades via `_stream()`)
- [ ] Cross-SDK parity validation (Python, Go, C++ return identical data)
- [ ] Auto-reconnect on FPSS disconnect
- [ ] Index price streaming
- [ ] Split/dividend endpoints (v3: "Coming Soon")
