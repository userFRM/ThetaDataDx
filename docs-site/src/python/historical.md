# Historical Data (Python)

All historical data is accessed through `DirectClient`. Every call runs through compiled Rust -- gRPC, protobuf parsing, zstd decompression, and FIT decoding all happen at native speed.

## Connecting

```python
from thetadatadx import Credentials, Config, DirectClient

creds = Credentials.from_file("creds.txt")
client = DirectClient(creds, Config.production())
```

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes.

## DataFrame Support

All data methods have `_df` variants that return pandas DataFrames directly:

```python
df = client.stock_history_eod_df("AAPL", "20240101", "20240301")
```

Or convert any result explicitly:

```python
from thetadatadx import to_dataframe

eod = client.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
```

Requires `pip install thetadatadx[pandas]`.

---

## Stock Endpoints (14)

### List

```python
# All available stock symbols
symbols = client.stock_list_symbols()

# Available dates by request type
dates = client.stock_list_dates("EOD", "AAPL")
```

### Snapshots

```python
# Latest OHLC snapshot (one or more symbols)
ticks = client.stock_snapshot_ohlc(["AAPL", "MSFT"])

# Latest trade snapshot
ticks = client.stock_snapshot_trade(["AAPL"])

# Latest NBBO quote snapshot
ticks = client.stock_snapshot_quote(["AAPL", "MSFT", "GOOGL"])

# Latest market value
result = client.stock_snapshot_market_value(["AAPL"])
```

### History

```python
# End-of-day data for a date range
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# As DataFrame
df = client.stock_history_eod_df("AAPL", "20240101", "20240301")
print(df.describe())

# Intraday OHLC bars (single date)
bars = client.stock_history_ohlc("AAPL", "20240315", "60000")
print(f"{len(bars)} bars")

# Intraday OHLC bars (date range)
bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000")

# All trades for a date
trades = client.stock_history_trade("AAPL", "20240315")
print(f"{len(trades)} trades")

# NBBO quotes at a given interval
quotes = client.stock_history_quote("AAPL", "20240315", "60000")
df = client.stock_history_quote_df("AAPL", "20240315", "0")

# Combined trade + quote ticks
result = client.stock_history_trade_quote("AAPL", "20240315")
```

### At-Time

```python
# Trade at a specific time of day across a date range
# time_of_day is milliseconds from midnight ET (34200000 = 9:30 AM)
trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000")

# Quote at a specific time of day
quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000")
```

---

## Option Endpoints (34)

### List

```python
# All option underlying symbols
symbols = client.option_list_symbols()

# Expiration dates for an underlying
exps = client.option_list_expirations("SPY")
print(exps[:10])

# Strike prices for an expiration
strikes = client.option_list_strikes("SPY", "20240419")
print(f"{len(strikes)} strikes")

# Available dates for a contract
dates = client.option_list_dates("EOD", "SPY", "20240419", "500000", "C")

# All contracts for a symbol on a date
contracts = client.option_list_contracts("EOD", "SPY", "20240315")
```

### Snapshots

```python
ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C")
trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C")
quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C")
oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C")
mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C")
```

### Snapshot Greeks

```python
# All Greeks at once
all_g = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C")

# By order
first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C")
second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C")
third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C")

# Just IV
iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C")
```

### History

```python
# End-of-day option data
eod = client.option_history_eod("SPY", "20240419", "500000", "C",
                                "20240101", "20240301")

# Intraday OHLC bars
bars = client.option_history_ohlc("SPY", "20240419", "500000", "C",
                                  "20240315", "60000")

# All trades
trades = client.option_history_trade("SPY", "20240419", "500000", "C", "20240315")

# NBBO quotes
quotes = client.option_history_quote("SPY", "20240419", "500000", "C",
                                     "20240315", "60000")

# Combined trade + quote ticks
result = client.option_history_trade_quote("SPY", "20240419", "500000", "C", "20240315")

# Open interest history
oi = client.option_history_open_interest("SPY", "20240419", "500000", "C", "20240315")
```

### History Greeks

```python
# EOD Greeks over a date range
greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500000", "C",
                                               "20240101", "20240301")

# Intraday Greeks sampled by interval
all_g = client.option_history_greeks_all("SPY", "20240419", "500000", "C",
                                          "20240315", "60000")
first = client.option_history_greeks_first_order("SPY", "20240419", "500000", "C",
                                                  "20240315", "60000")
second = client.option_history_greeks_second_order("SPY", "20240419", "500000", "C",
                                                    "20240315", "60000")
third = client.option_history_greeks_third_order("SPY", "20240419", "500000", "C",
                                                  "20240315", "60000")
iv_hist = client.option_history_greeks_implied_volatility("SPY", "20240419", "500000", "C",
                                                           "20240315", "60000")
```

### Trade Greeks

Greeks computed on each individual trade:

```python
all_tg = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C", "20240315")
first_tg = client.option_history_trade_greeks_first_order("SPY", "20240419", "500000", "C", "20240315")
second_tg = client.option_history_trade_greeks_second_order("SPY", "20240419", "500000", "C", "20240315")
third_tg = client.option_history_trade_greeks_third_order("SPY", "20240419", "500000", "C", "20240315")
iv_tg = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500000", "C", "20240315")
```

### At-Time

```python
trades = client.option_at_time_trade("SPY", "20240419", "500000", "C",
                                     "20240101", "20240301", "34200000")
quotes = client.option_at_time_quote("SPY", "20240419", "500000", "C",
                                     "20240101", "20240301", "34200000")
```

---

## Index Endpoints (9)

```python
# List
symbols = client.index_list_symbols()
dates = client.index_list_dates("SPX")

# Snapshots
ohlc = client.index_snapshot_ohlc(["SPX", "NDX"])
price = client.index_snapshot_price(["SPX", "NDX"])
mv = client.index_snapshot_market_value(["SPX"])

# History
eod = client.index_history_eod("SPX", "20240101", "20240301")
df = client.index_history_eod_df("SPX", "20240101", "20240301")
bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000")
price = client.index_history_price("SPX", "20240315", "60000")

# At-Time
result = client.index_at_time_price("SPX", "20240101", "20240301", "34200000")
```

---

## Rate Endpoints (1)

```python
result = client.interest_rate_history_eod("SOFR", "20240101", "20240301")
```

Available rate symbols: `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`, `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`, `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.

---

## Calendar Endpoints (3)

```python
result = client.calendar_open_today()
result = client.calendar_on_date("20240315")
result = client.calendar_year("2024")
```

---

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |
