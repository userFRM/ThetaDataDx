# API Reference (Python)

Complete method listing for the `thetadatadx` Python package. Every call runs through compiled Rust via PyO3.

## Credentials

```python
# From file (line 1 = email, line 2 = password)
creds = Credentials.from_file("creds.txt")

# Direct construction
creds = Credentials("user@example.com", "password")
```

## Config

```python
config = Config.production()  # ThetaData NJ production servers
config = Config.dev()         # dev servers with shorter timeouts
```

## ThetaDataDx

```python
tdx = ThetaDataDx(creds, Config.production())
```

All methods return lists of dicts. All methods also have `_df` variants returning pandas DataFrames.

### Stock Methods (14)

| Method | Description |
|--------|-------------|
| `stock_list_symbols()` | All stock symbols |
| `stock_list_dates(request_type, symbol)` | Available dates by request type |
| `stock_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `stock_snapshot_trade(symbols)` | Latest trade snapshot |
| `stock_snapshot_quote(symbols)` | Latest NBBO quote snapshot |
| `stock_snapshot_market_value(symbols)` | Latest market value |
| `stock_history_eod(symbol, start, end)` | End-of-day data |
| `stock_history_ohlc(symbol, date, interval)` | Intraday OHLC bars |
| `stock_history_ohlc_range(symbol, start, end, interval)` | OHLC bars across date range |
| `stock_history_trade(symbol, date)` | All trades for a date |
| `stock_history_quote(symbol, date, interval)` | NBBO quotes |
| `stock_history_trade_quote(symbol, date)` | Combined trade+quote |
| `stock_at_time_trade(symbol, start, end, time)` | Trade at specific time |
| `stock_at_time_quote(symbol, start, end, time)` | Quote at specific time |

### Option Methods (34)

| Method | Description |
|--------|-------------|
| `option_list_symbols()` | Option underlying symbols |
| `option_list_dates(request_type, symbol, exp, strike, right)` | Available dates |
| `option_list_expirations(symbol)` | Expiration dates |
| `option_list_strikes(symbol, exp)` | Strike prices |
| `option_list_contracts(request_type, symbol, date)` | All contracts for a date |
| `option_snapshot_ohlc(symbol, exp, strike, right)` | Latest OHLC |
| `option_snapshot_trade(symbol, exp, strike, right)` | Latest trade |
| `option_snapshot_quote(symbol, exp, strike, right)` | Latest quote |
| `option_snapshot_open_interest(symbol, exp, strike, right)` | Latest OI |
| `option_snapshot_market_value(symbol, exp, strike, right)` | Latest market value |
| `option_snapshot_greeks_implied_volatility(symbol, exp, strike, right)` | IV snapshot |
| `option_snapshot_greeks_all(symbol, exp, strike, right)` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(symbol, exp, strike, right)` | First-order Greeks |
| `option_snapshot_greeks_second_order(symbol, exp, strike, right)` | Second-order Greeks |
| `option_snapshot_greeks_third_order(symbol, exp, strike, right)` | Third-order Greeks |
| `option_history_eod(symbol, exp, strike, right, start, end)` | EOD data |
| `option_history_ohlc(symbol, exp, strike, right, date, interval)` | Intraday OHLC |
| `option_history_trade(symbol, exp, strike, right, date)` | All trades |
| `option_history_quote(symbol, exp, strike, right, date, interval)` | NBBO quotes |
| `option_history_trade_quote(symbol, exp, strike, right, date)` | Combined trade+quote |
| `option_history_open_interest(symbol, exp, strike, right, date)` | OI history |
| `option_history_greeks_eod(symbol, exp, strike, right, start, end)` | EOD Greeks |
| `option_history_greeks_all(symbol, exp, strike, right, date, interval)` | All Greeks history |
| `option_history_trade_greeks_all(symbol, exp, strike, right, date)` | Greeks on each trade |
| `option_history_greeks_first_order(symbol, exp, strike, right, date, interval)` | First-order history |
| `option_history_trade_greeks_first_order(symbol, exp, strike, right, date)` | First-order on each trade |
| `option_history_greeks_second_order(symbol, exp, strike, right, date, interval)` | Second-order history |
| `option_history_trade_greeks_second_order(symbol, exp, strike, right, date)` | Second-order on each trade |
| `option_history_greeks_third_order(symbol, exp, strike, right, date, interval)` | Third-order history |
| `option_history_trade_greeks_third_order(symbol, exp, strike, right, date)` | Third-order on each trade |
| `option_history_greeks_implied_volatility(symbol, exp, strike, right, date, interval)` | IV history |
| `option_history_trade_greeks_implied_volatility(symbol, exp, strike, right, date)` | IV on each trade |
| `option_at_time_trade(symbol, exp, strike, right, start, end, time)` | Trade at specific time |
| `option_at_time_quote(symbol, exp, strike, right, start, end, time)` | Quote at specific time |

### Index Methods (9)

| Method | Description |
|--------|-------------|
| `index_list_symbols()` | All index symbols |
| `index_list_dates(symbol)` | Available dates |
| `index_snapshot_ohlc(symbols)` | Latest OHLC |
| `index_snapshot_price(symbols)` | Latest price |
| `index_snapshot_market_value(symbols)` | Latest market value |
| `index_history_eod(symbol, start, end)` | EOD data |
| `index_history_ohlc(symbol, start, end, interval)` | Intraday OHLC |
| `index_history_price(symbol, date, interval)` | Intraday price |
| `index_at_time_price(symbol, start, end, time)` | Price at specific time |

### Calendar Methods (3)

| Method | Description |
|--------|-------------|
| `calendar_open_today()` | Is the market open today? |
| `calendar_on_date(date)` | Calendar info for a date |
| `calendar_year(year)` | Calendar for an entire year |

### Rate Methods (1)

| Method | Description |
|--------|-------------|
| `interest_rate_history_eod(symbol, start, end)` | Interest rate EOD history |

## DataFrame Support

### to_dataframe

Convert any result list to a pandas DataFrame:

```python
from thetadatadx import to_dataframe

eod = client.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
```

### _df Methods

All 61 data methods have `_df` variants:

```python
df = client.stock_history_eod_df("AAPL", "20240101", "20240301")
df = client.stock_history_ohlc_df("AAPL", "20240315", "60000")
df = client.option_history_eod_df("SPY", "20240419", "500000", "C",
                                  "20240101", "20240301")
```

Requires `pip install thetadatadx[pandas]`.

### to_polars

```python
from thetadatadx import to_polars

eod = client.stock_history_eod("AAPL", "20240101", "20240301")
df = to_polars(eod)
```

Requires `pip install thetadatadx[polars]`.

## Streaming (via ThetaDataDx)

```python
tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
event = tdx.next_event(timeout_ms=5000)
tdx.stop_streaming()
```

| Method | Description |
|--------|-------------|
| `start_streaming()` | Connect to FPSS streaming servers |
| `subscribe_quotes(symbol)` | Subscribe to quote data |
| `subscribe_trades(symbol)` | Subscribe to trade data |
| `next_event(timeout_ms=5000)` | Poll next event (dict or `None`) |
| `stop_streaming()` | Graceful shutdown of streaming |

## Greeks Functions

```python
from thetadatadx import all_greeks, implied_volatility

# All 22 Greeks (returns dict)
g = all_greeks(spot, strike, rate, div_yield, tte, option_price, is_call)

# Just IV (returns tuple)
iv, err = implied_volatility(spot, strike, rate, div_yield, tte, option_price, is_call)
```

### all_greeks Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `spot` | `float` | Underlying price |
| `strike` | `float` | Strike price |
| `rate` | `float` | Risk-free rate |
| `div_yield` | `float` | Dividend yield |
| `tte` | `float` | Time to expiration (years) |
| `option_price` | `float` | Market price of the option |
| `is_call` | `bool` | `True` for call, `False` for put |

### Return Dict Keys

`value`, `delta`, `gamma`, `theta`, `vega`, `rho`, `iv`, `iv_error`, `vanna`, `charm`, `vomma`, `veta`, `speed`, `zomma`, `color`, `ultima`, `d1`, `d2`, `dual_delta`, `dual_gamma`, `epsilon`, `lambda`
