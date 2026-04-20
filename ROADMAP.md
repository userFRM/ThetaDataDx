# ThetaDataDx Roadmap

## Endpoint Status

Last validated: 2026-04-20 against live MDDS production.
Validator: `scripts/validate_cli.py` — full parameter-mode matrix, 134 cells.
Result: **127 PASS / 7 SKIP / 0 FAIL**. All seven SKIPs are account-tier
limits (this account holds Options Pro + Stock Standard; it has no
Standard-tier Index subscription and no Value-tier interest-rate or
market-value subscription). Every endpoint the account can reach PASSes
on both the concrete-parameter path and every wildcard / bulk-chain
variant covered by the matrix.

Run locally with:
```bash
python3 scripts/validate_cli.py /path/to/creds.txt
```
Artifact: `artifacts/validator_cli.json`.

### Stock

| Endpoint | Tier | Status |
|----------|------|--------|
| `stock_list_symbols()` | Free | Verified |
| `stock_list_dates(req, sym)` | Free | Verified |
| `stock_history_eod(sym, start, end)` | Standard | Verified |
| `stock_history_ohlc(sym, date, interval)` | Standard | Verified |
| `stock_history_ohlc_range(sym, start, end, interval)` | Standard | Verified |
| `stock_history_trade(sym, date)` | Standard | Verified |
| `stock_history_quote(sym, date)` | Standard | Verified |
| `stock_history_trade_quote(sym, date)` | Standard | Verified |
| `stock_snapshot_ohlc(sym)` | Standard | Verified |
| `stock_snapshot_trade(sym)` | Standard | Verified |
| `stock_snapshot_quote(sym)` | Standard | Verified |
| `stock_snapshot_market_value(sym)` | Value | Not tested (account lacks tier) |
| `stock_at_time_trade(sym, date, time)` | Standard | Verified |
| `stock_at_time_quote(sym, date, time)` | Standard | Verified |

### Option

| Endpoint | Tier | Status |
|----------|------|--------|
| `option_list_symbols()` | Free | Verified |
| `option_list_contracts(req, sym, date)` | Standard | Verified |
| `option_list_expirations(sym)` | Standard | Verified |
| `option_list_strikes(sym, exp)` | Standard | Verified |
| `option_list_dates(req, sym, exp, strike, right)` | Standard | Verified |
| `option_history_eod(...)` | Standard | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_ohlc(...)` | Standard | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_trade(...)` | Standard | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_quote(...)` | Standard | Verified (concrete + bulk_chain + all_strikes_one_exp) |
| `option_history_trade_quote(...)` | Standard | Verified |
| `option_history_open_interest(...)` | Standard | Verified |
| `option_snapshot_ohlc(...)` | Standard | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_trade(...)` | Standard | Verified |
| `option_snapshot_quote(...)` | Standard | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_open_interest(...)` | Standard | Verified |
| `option_snapshot_market_value(...)` | Standard | Verified |
| `option_snapshot_greeks_iv(...)` | Pro | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_greeks_all(...)` | Pro | Verified (concrete + all_exps_one_strike) |
| `option_snapshot_greeks_first_order(...)` | Pro | Verified |
| `option_snapshot_greeks_second_order(...)` | Pro | Verified |
| `option_snapshot_greeks_third_order(...)` | Pro | Verified |
| `option_history_greeks_eod(...)` | Pro | Verified |
| `option_history_greeks_iv(...)` | Pro | Verified |
| `option_history_greeks_all(...)` | Pro | Verified |
| `option_history_greeks_first_order(...)` | Pro | Verified |
| `option_history_greeks_second_order(...)` | Pro | Verified |
| `option_history_greeks_third_order(...)` | Pro | Verified |
| `option_history_trade_greeks_iv(...)` | Pro | Verified |
| `option_history_trade_greeks_all(...)` | Pro | Verified |
| `option_history_trade_greeks_first_order(...)` | Pro | Verified |
| `option_history_trade_greeks_second_order(...)` | Pro | Verified |
| `option_history_trade_greeks_third_order(...)` | Pro | Verified |
| `option_at_time_trade(...)` | Standard | Verified |
| `option_at_time_quote(...)` | Standard | Verified |

### Index

| Endpoint | Tier | Status |
|----------|------|--------|
| `index_list_symbols()` | Free | Verified |
| `index_list_dates(req, sym)` | Free | Verified |
| `index_snapshot_ohlc(sym)` | Standard | Not tested (account lacks tier) |
| `index_snapshot_price(sym)` | Standard | Not tested (account lacks tier) |
| `index_snapshot_market_value(sym)` | Standard | Not tested (account lacks tier) |
| `index_history_eod(sym, start, end)` | Free | Verified |
| `index_history_ohlc(...)` | Standard | Not tested (account lacks tier) |
| `index_history_price(...)` | Value | Not tested (account lacks tier) |
| `index_at_time_price(...)` | Standard | Not tested (account lacks tier) |

### Calendar, Interest Rate, Utility

| Endpoint | Tier | Status |
|----------|------|--------|
| `interest_rate_history_eod(sym, start, end)` | Value | Not tested (account lacks tier) |
| `calendar_year(year)` | Free | Verified |
| `calendar_on_date(date)` | Free | Verified |
| `calendar_open_today()` | Free | Verified |

### FPSS Streaming

| Feature | Tier | Status |
|---------|------|--------|
| Stock quote subscription | Standard | Verified (prod + dev) |
| Stock trade subscription | Standard | Verified (prod + dev) |
| Option quote subscription | Standard | Verified |
| Option trade subscription | Standard | Verified |
| Open interest subscription | Pro | Verified |
| Full trade firehose (Option) | Pro | Verified (prod 5-minute capture, 22+ subs) |
| Full trade firehose (Stock) | Pro | Verified |
| Full OI firehose | Pro | Verified |
| Index price subscription | Free | Not tested (account lacks dedicated Index data) |
| Dev server replay | -- | Verified |
| Reconnection | -- | Verified (Java-parity per-read deadline, PR #370) |
| Mid-frame TCP pause tolerance | -- | Verified (prod 5-min, zero fatal events) |

## Remaining Work

### Blocked by subscription tier (not a bug — account limits)

- [ ] Test stock/index/interest-rate endpoints that require Value or
  dedicated-Index subscriptions. Current account profile: Options Pro +
  Stock Standard + Indices Free.

### Open

- [ ] Cross-SDK parity validation (Python, TypeScript/Node.js, Go, C++
  return identical data for every endpoint the validator covers).
- [ ] Split / dividend endpoints (v3: "Coming Soon" upstream).

### Recently closed (this cycle)

- [x] Java-parity mid-frame retry, eliminates the reconnect-storm class
  of issues tracked through #192 / #369 (PR #370).
- [x] Options Pro endpoint coverage — every `option_*_greeks_*` and
  `option_*_trade_greeks_*` variant now PASSes on prod.
- [x] Typed SDK surface for Python and TypeScript (pyclass / napi
  `#[napi(object)]` with `BigInt` for u64 fields). See CHANGELOG v7.3.1.
- [x] Auto-reconnect on FPSS disconnect.
- [x] Large data streaming via `_stream()` helpers.
