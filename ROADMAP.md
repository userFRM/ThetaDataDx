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

| Feature | Tier | Status |
|---------|------|--------|
| Stock quote subscription | Standard | Verified |
| Stock trade subscription | Standard | Verified |
| Option quote subscription | Standard | Verified |
| Option trade subscription | Standard | Verified |
| Open interest subscription | Standard | Verified |
| Full trade firehose | Standard | Verified |
| Full OI firehose | Standard | Not tested |
| Index price subscription | Free | Not tested |
| Dev server replay | -- | Verified |
| Reconnection | -- | Verified |

## Remaining Work

### High Priority

- [x] Test FPSS streaming subscriptions (stock, option, OI, firehose -- all verified on dev server)
- [ ] Test Pro-tier endpoints when subscription available
- [ ] Cross-SDK parity validation (Python, Go, C++ return identical data)

### Medium Priority

- [ ] Auto-reconnect on FPSS disconnect
- [ ] Index streaming validation

### Low Priority

- [ ] Split/dividend endpoints (v3: "Coming Soon")
