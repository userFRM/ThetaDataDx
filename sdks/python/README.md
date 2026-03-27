# thetadatadx (Python)

Python SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via PyO3.

**This is NOT a Python reimplementation.** Every call goes through compiled Rust -- gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming all happen at native speed. Python is just the interface.

## Installation

```bash
pip install thetadatadx

# With pandas DataFrame support
pip install thetadatadx[pandas]
```

Or build from source (requires Rust toolchain):

```bash
pip install maturin
maturin develop --release
```

## Quick Start

```python
from thetadatadx import Credentials, Config, ThetaDataDx

# Authenticate and connect
creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Fetch end-of-day data
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} H={tick['high']:.2f} "
          f"L={tick['low']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# Intraday 1-minute OHLC bars
bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000")
print(f"{len(bars)} bars")

# Option chain
exps = tdx.option_list_expirations("SPY")
strikes = tdx.option_list_strikes("SPY", exps[0])
```

## Greeks Calculator

Full Black-Scholes calculator with 22 Greeks, running in Rust:

```python
from thetadatadx import all_greeks, implied_volatility

# All Greeks at once
g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05, div_yield=0.015,
    tte=30/365, option_price=8.50, is_call=True
)
print(f"IV={g['iv']:.4f} Delta={g['delta']:.4f} Gamma={g['gamma']:.6f}")

# Just IV
iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30/365, 8.50, True)
```

## API

### `Credentials`
- `Credentials(email, password)` -- direct construction
- `Credentials.from_file(path)` -- load from creds.txt

### `Config`
- `Config.production()` -- ThetaData NJ production servers
- `Config.dev()` -- dev servers with shorter timeouts

### `ThetaDataDx(creds, config)`

All 61 endpoints are available. Methods return lists of dicts.

#### Stock Methods (14)

| Method | Description |
|--------|-------------|
| `stock_list_symbols()` | All stock symbols |
| `stock_list_dates(request_type, symbol)` | Available dates by request type |
| `stock_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `stock_snapshot_trade(symbols)` | Latest trade snapshot |
| `stock_snapshot_quote(symbols)` | Latest NBBO quote snapshot |
| `stock_snapshot_market_value(symbols)` | Latest market value snapshot |
| `stock_history_eod(symbol, start, end)` | End-of-day data |
| `stock_history_ohlc(symbol, date, interval)` | Intraday OHLC bars |
| `stock_history_ohlc_range(symbol, start, end, interval)` | OHLC bars across date range |
| `stock_history_trade(symbol, date)` | All trades for a date |
| `stock_history_quote(symbol, date, interval)` | NBBO quotes |
| `stock_history_trade_quote(symbol, date)` | Combined trade+quote ticks |
| `stock_at_time_trade(symbol, start, end, time)` | Trade at specific time across dates |
| `stock_at_time_quote(symbol, start, end, time)` | Quote at specific time across dates |

#### Option Methods (34)

| Method | Description |
|--------|-------------|
| `option_list_symbols()` | Option underlying symbols |
| `option_list_dates(request_type, symbol, exp, strike, right)` | Available dates for a contract |
| `option_list_expirations(symbol)` | Expiration dates |
| `option_list_strikes(symbol, exp)` | Strike prices |
| `option_list_contracts(request_type, symbol, date)` | All contracts for a date |
| `option_snapshot_ohlc(symbol, exp, strike, right)` | Latest OHLC snapshot |
| `option_snapshot_trade(symbol, exp, strike, right)` | Latest trade snapshot |
| `option_snapshot_quote(symbol, exp, strike, right)` | Latest quote snapshot |
| `option_snapshot_open_interest(symbol, exp, strike, right)` | Latest open interest |
| `option_snapshot_market_value(symbol, exp, strike, right)` | Latest market value |
| `option_snapshot_greeks_implied_volatility(symbol, exp, strike, right)` | IV snapshot |
| `option_snapshot_greeks_all(symbol, exp, strike, right)` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(symbol, exp, strike, right)` | First-order Greeks |
| `option_snapshot_greeks_second_order(symbol, exp, strike, right)` | Second-order Greeks |
| `option_snapshot_greeks_third_order(symbol, exp, strike, right)` | Third-order Greeks |
| `option_history_eod(symbol, exp, strike, right, start, end)` | EOD option data |
| `option_history_ohlc(symbol, exp, strike, right, date, interval)` | Intraday OHLC bars |
| `option_history_trade(symbol, exp, strike, right, date)` | All trades |
| `option_history_quote(symbol, exp, strike, right, date, interval)` | NBBO quotes |
| `option_history_trade_quote(symbol, exp, strike, right, date)` | Combined trade+quote |
| `option_history_open_interest(symbol, exp, strike, right, date)` | Open interest history |
| `option_history_greeks_eod(symbol, exp, strike, right, start, end)` | EOD Greeks |
| `option_history_greeks_all(symbol, exp, strike, right, date, interval)` | All Greeks history |
| `option_history_trade_greeks_all(symbol, exp, strike, right, date)` | Greeks on each trade |
| `option_history_greeks_first_order(symbol, exp, strike, right, date, interval)` | First-order Greeks history |
| `option_history_trade_greeks_first_order(symbol, exp, strike, right, date)` | First-order on each trade |
| `option_history_greeks_second_order(symbol, exp, strike, right, date, interval)` | Second-order Greeks history |
| `option_history_trade_greeks_second_order(symbol, exp, strike, right, date)` | Second-order on each trade |
| `option_history_greeks_third_order(symbol, exp, strike, right, date, interval)` | Third-order Greeks history |
| `option_history_trade_greeks_third_order(symbol, exp, strike, right, date)` | Third-order on each trade |
| `option_history_greeks_implied_volatility(symbol, exp, strike, right, date, interval)` | IV history |
| `option_history_trade_greeks_implied_volatility(symbol, exp, strike, right, date)` | IV on each trade |
| `option_at_time_trade(symbol, exp, strike, right, start, end, time)` | Trade at specific time |
| `option_at_time_quote(symbol, exp, strike, right, start, end, time)` | Quote at specific time |

#### Index Methods (9)

| Method | Description |
|--------|-------------|
| `index_list_symbols()` | All index symbols |
| `index_list_dates(symbol)` | Available dates for an index |
| `index_snapshot_ohlc(symbol)` | Latest OHLC snapshot |
| `index_snapshot_price(symbol)` | Latest price snapshot |
| `index_snapshot_market_value(symbol)` | Latest market value snapshot |
| `index_history_eod(symbol, start, end)` | End-of-day index data |
| `index_history_ohlc(symbol, start, end, interval)` | Intraday OHLC bars |
| `index_history_price(symbol, date, interval)` | Intraday price history |
| `index_at_time_price(symbol, start, end, time)` | Price at specific time |

#### Calendar Methods (3)

| Method | Description |
|--------|-------------|
| `calendar_open_today()` | Is the market open today? |
| `calendar_on_date(date)` | Calendar info for a date |
| `calendar_year(year)` | Calendar for an entire year |

#### Rate Methods (1)

| Method | Description |
|--------|-------------|
| `interest_rate_history_eod(symbol, start, end)` | Interest rate EOD history |

### Streaming (via `ThetaDataDx`)
Real-time streaming is accessed through the same `ThetaDataDx` instance.

#### Per-contract subscriptions (stocks)

| Method | Description |
|--------|-------------|
| `subscribe_quotes(symbol)` | Subscribe to quote data for a stock |
| `subscribe_trades(symbol)` | Subscribe to trade data for a stock |
| `subscribe_open_interest(symbol)` | Subscribe to open interest data for a stock |
| `unsubscribe_quotes(symbol)` | Unsubscribe from quote data for a stock |
| `unsubscribe_trades(symbol)` | Unsubscribe from trade data for a stock |
| `unsubscribe_open_interest(symbol)` | Unsubscribe from open interest data for a stock |

#### Per-contract subscriptions (options)

| Method | Description |
|--------|-------------|
| `subscribe_option_quotes(symbol, exp_date, is_call, strike)` | Subscribe to option quote data |
| `subscribe_option_trades(symbol, exp_date, is_call, strike)` | Subscribe to option trade data |
| `subscribe_option_open_interest(symbol, exp_date, is_call, strike)` | Subscribe to option OI data |
| `unsubscribe_option_quotes(symbol, exp_date, is_call, strike)` | Unsubscribe from option quotes |
| `unsubscribe_option_trades(symbol, exp_date, is_call, strike)` | Unsubscribe from option trades |

#### Full-type subscriptions

| Method | Description |
|--------|-------------|
| `subscribe_full_trades(sec_type)` | Subscribe to ALL trades for a security type (`"STOCK"`, `"OPTION"`, `"INDEX"`) |

**Full trade stream behavior:** When subscribed via `subscribe_full_trades("OPTION")`, the ThetaData FPSS server sends a **bundle** for every trade across ALL option contracts:

1. Pre-trade NBBO quote
2. OHLC bar for the traded contract
3. The trade itself
4. Two post-trade NBBO quotes

Events arrive as a mix of `quote`, `trade`, and `ohlcvc` kinds. Use `contract_id` to identify which contract each event belongs to, and filter on `kind` to select the data types you care about:

```python
tdx.start_streaming()
tdx.subscribe_full_trades("OPTION")

# Build a contract ID -> symbol map as assignments arrive
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=100)
    if event is None:
        continue

    # Track contract assignments
    if event["kind"] == "contract_assigned":
        contracts[event["id"]] = event["detail"]
        continue

    contract = contracts.get(event.get("contract_id"), "unknown")

    # Filter by type -- you choose what you want
    if event["kind"] == "trade":
        print(f"[{contract}] TRADE {event['price']:.2f} x {event['size']}")
    elif event["kind"] == "quote":
        print(f"[{contract}] QUOTE bid={event['bid']:.2f} ask={event['ask']:.2f}")
    # Skip ohlcvc if you don't need bars

tdx.stop_streaming()
```

You can also subscribe to per-contract streams if you only need specific symbols rather than the full firehose.

#### State & lifecycle

| Method | Description |
|--------|-------------|
| `contract_map()` | Get dict mapping contract IDs to string descriptions |
| `contract_lookup(id)` | Look up a single contract by ID (returns str or None) |
| `active_subscriptions()` | Get list of active subscriptions (list of dicts with "kind" and "contract") |
| `next_event(timeout_ms=5000)` | Poll for the next event (returns dict or None on timeout) |
| `shutdown()` | Graceful shutdown |

### `to_dataframe(data)`
Convert a list of tick dicts to a pandas DataFrame. Requires `pip install thetadatadx[pandas]`.

### `_df` method variants
All 61 `ThetaDataDx` data methods have `_df` variants that return DataFrames directly:
`stock_history_eod_df()`, `stock_history_ohlc_df()`, `option_list_expirations_df()`, `index_history_eod_df()`, etc.

### `all_greeks(spot, strike, rate, div_yield, tte, price, is_call)`
Returns dict with 22 Greeks: delta, gamma, theta, vega, rho, iv, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda.

### `implied_volatility(spot, strike, rate, div_yield, tte, price, is_call)`
Returns `(iv, error)` tuple.

## Architecture

```mermaid
graph TD
    A["Python code"] -- "PyO3 FFI" --> B["thetadatadx Rust crate"]
    B -- "tonic gRPC / TLS TCP" --> C["ThetaData servers"]
```

No HTTP middleware, no Java terminal, no subprocess. Direct wire protocol access at Rust speed.

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers:

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Start streaming and subscribe to real-time data
tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("SPY")

# Poll for events
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        break  # timeout, no event received
    if event["kind"] == "quote":
        print(f"Quote: {event['contract']} bid={event['bid']} ask={event['ask']}")
    elif event["kind"] == "trade":
        print(f"Trade: {event['contract']} price={event['price']} size={event['size']}")

tdx.stop_streaming()
```

## pandas DataFrame Conversion

Convert any result to a pandas DataFrame:

```python
from thetadatadx import Credentials, Config, ThetaDataDx, to_dataframe

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Option 1: convert an existing result
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
print(df.head())

# Option 2: use _df convenience methods
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
df = tdx.stock_history_ohlc_df("AAPL", "20240315", "60000")
df = tdx.option_list_expirations_df("SPY")
```

Install with: `pip install thetadatadx[pandas]`
