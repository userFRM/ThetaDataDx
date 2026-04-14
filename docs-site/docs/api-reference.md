---
title: API Reference
description: Complete API reference for the ThetaDataDx SDK covering all endpoints, types, and Greeks functions across Rust, Python, Go, and C++.
---

# API Reference

ThetaDataDx provides a unified client for accessing ThetaData market data. Historical data flows over MDDS/gRPC; real-time streaming flows over FPSS/TCP. The SDK ships native bindings for Rust, Python, Go, and C++, all backed by the same compiled Rust core.

**61 typed endpoints** + 4 streaming variants + 22 Greeks functions + IV solver.

## Client Construction

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())
```
```go [Go]
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
defer client.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
```
:::

---

## Stock Endpoints

### stock_list_symbols

List all available stock ticker symbols.

::: code-group
```rust [Rust]
let symbols = tdx.stock_list_symbols().await?;
```
```python [Python]
symbols = tdx.stock_list_symbols()
```
```go [Go]
symbols, err := client.StockListSymbols()
```
```cpp [C++]
auto symbols = client.stock_list_symbols();
```
:::

**Parameters:** None

**Returns:** List of ticker symbol strings.

---

### stock_list_dates

List available dates for a stock by request type.

::: code-group
```rust [Rust]
let dates = tdx.stock_list_dates("TRADE", "AAPL").await?;
```
```python [Python]
dates = tdx.stock_list_dates("TRADE", "AAPL")
```
```go [Go]
dates, err := client.StockListDates("TRADE", "AAPL")
```
```cpp [C++]
auto dates = client.stock_list_dates("TRADE", "AAPL");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type: `"TRADE"`, `"QUOTE"`, `"OHLC"` |
| `symbol` | string | Yes | Ticker symbol |

**Returns:** List of date strings (`YYYYMMDD`).

---

### stock_snapshot_ohlc

Latest OHLC snapshot for one or more stocks.

::: code-group
```rust [Rust]
let bars = tdx.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;
```
```python [Python]
bars = tdx.stock_snapshot_ohlc(["AAPL", "MSFT"])
```
```go [Go]
bars, err := client.StockSnapshotOHLC([]string{"AAPL", "MSFT"})
```
```cpp [C++]
auto bars = client.stock_snapshot_ohlc({"AAPL", "MSFT"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [OhlcTick](#ohlctick).

---

### stock_snapshot_trade

Latest trade snapshot for one or more stocks.

::: code-group
```rust [Rust]
let trades = tdx.stock_snapshot_trade(&["AAPL"]).await?;
```
```python [Python]
trades = tdx.stock_snapshot_trade(["AAPL"])
```
```go [Go]
trades, err := client.StockSnapshotTrade([]string{"AAPL"})
```
```cpp [C++]
auto trades = client.stock_snapshot_trade({"AAPL"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [TradeTick](#tradetick).

---

### stock_snapshot_quote

Latest NBBO quote snapshot for one or more stocks.

::: code-group
```rust [Rust]
let quotes = tdx.stock_snapshot_quote(&["AAPL"]).await?;
```
```python [Python]
quotes = tdx.stock_snapshot_quote(["AAPL"])
```
```go [Go]
quotes, err := client.StockSnapshotQuote([]string{"AAPL"})
```
```cpp [C++]
auto quotes = client.stock_snapshot_quote({"AAPL"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [QuoteTick](#quotetick).

---

### stock_snapshot_market_value

Latest market value snapshot for one or more stocks.

::: code-group
```rust [Rust]
let mv = tdx.stock_snapshot_market_value(&["AAPL"]).await?;
```
```python [Python]
mv = tdx.stock_snapshot_market_value(["AAPL"])
```
```go [Go]
mv, err := client.StockSnapshotMarketValue([]string{"AAPL"})
```
```cpp [C++]
auto mv = client.stock_snapshot_market_value({"AAPL"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** Array of MarketValueTick records with market cap, shares outstanding, enterprise value, book value, free float.

---

### stock_history_eod

End-of-day stock data across a date range.

::: code-group
```rust [Rust]
let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
```
```python [Python]
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
```
```go [Go]
eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
```
```cpp [C++]
auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

**Returns:** List of [EodTick](#eodtick).

---

### stock_history_ohlc

Intraday OHLC bars for a single date.

::: code-group
```rust [Rust]
let bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000").await?;
```
```python [Python]
bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000")
```
```go [Go]
bars, err := client.StockHistoryOHLC("AAPL", "20240315", "60000")
```
```cpp [C++]
auto bars = client.stock_history_ohlc("AAPL", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `venue` | string | No | Data venue filter |

**Returns:** List of [OhlcTick](#ohlctick).

---

### stock_history_ohlc_range

Intraday OHLC bars across a date range.

::: code-group
```rust [Rust]
let bars = tdx.stock_history_ohlc_range("AAPL", "20240101", "20240301", "60000").await?;
```
```python [Python]
bars = tdx.stock_history_ohlc_range("AAPL", "20240101", "20240301", "60000")
```
```go [Go]
bars, err := client.StockHistoryOHLCRange("AAPL", "20240101", "20240301", "60000")
```
```cpp [C++]
auto bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |

**Returns:** List of [OhlcTick](#ohlctick).

---

### stock_history_trade

All trades for a stock on a given date.

::: code-group
```rust [Rust]
let trades = tdx.stock_history_trade("AAPL", "20240315").await?;
```
```python [Python]
trades = tdx.stock_history_trade("AAPL", "20240315")
```
```go [Go]
trades, err := client.StockHistoryTrade("AAPL", "20240315")
```
```cpp [C++]
auto trades = client.stock_history_trade("AAPL", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `venue` | string | No | Data venue filter |

**Returns:** List of [TradeTick](#tradetick).

**Tier:** Standard+

---

### stock_history_quote

NBBO quotes for a stock at a given interval.

::: code-group
```rust [Rust]
let quotes = tdx.stock_history_quote("AAPL", "20240315", "60000").await?;
```
```python [Python]
quotes = tdx.stock_history_quote("AAPL", "20240315", "60000")
```
```go [Go]
quotes, err := client.StockHistoryQuote("AAPL", "20240315", "60000")
```
```cpp [C++]
auto quotes = client.stock_history_quote("AAPL", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. Use `"0"` for every change. |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `venue` | string | No | Data venue filter |

**Returns:** List of [QuoteTick](#quotetick).

**Tier:** Standard+

---

### stock_history_trade_quote

Combined trade + quote ticks for a stock on a given date.

::: code-group
```rust [Rust]
let tq = tdx.stock_history_trade_quote("AAPL", "20240315").await?;
```
```python [Python]
tq = tdx.stock_history_trade_quote("AAPL", "20240315")
```
```go [Go]
tq, err := client.StockHistoryTradeQuote("AAPL", "20240315")
```
```cpp [C++]
auto tq = client.stock_history_trade_quote("AAPL", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `exclusive` | bool | No | Exclusive time bounds |
| `venue` | string | No | Data venue filter |

**Returns:** Array of TradeQuoteTick records with combined trade + quote fields.

**Tier:** Pro

---

### stock_at_time_trade

Trade at a specific time of day across a date range.

::: code-group
```rust [Rust]
let trades = tdx.stock_at_time_trade("AAPL", "20240101", "20240301", "09:30:00.000").await?;
```
```python [Python]
trades = tdx.stock_at_time_trade("AAPL", "20240101", "20240301", "09:30:00.000")
```
```go [Go]
trades, err := client.StockAtTimeTrade("AAPL", "20240101", "20240301", "09:30:00.000")
```
```cpp [C++]
auto trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "09:30:00.000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS` (e.g. `"09:30:00.000"`; legacy `"34200000"` also accepted) |
| `venue` | string | No | Data venue filter |

**Returns:** List of [TradeTick](#tradetick), one per date.

---

### stock_at_time_quote

Quote at a specific time of day across a date range.

::: code-group
```rust [Rust]
let quotes = tdx.stock_at_time_quote("AAPL", "20240101", "20240301", "09:30:00.000").await?;
```
```python [Python]
quotes = tdx.stock_at_time_quote("AAPL", "20240101", "20240301", "09:30:00.000")
```
```go [Go]
quotes, err := client.StockAtTimeQuote("AAPL", "20240101", "20240301", "09:30:00.000")
```
```cpp [C++]
auto quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "09:30:00.000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS` |
| `venue` | string | No | Data venue filter |

**Returns:** List of [QuoteTick](#quotetick), one per date.

---

## Option Endpoints

All option endpoints that operate on a specific contract require the contract spec parameters: `symbol`, `expiration`, `strike`, and `right`.

- `symbol` - Underlying ticker (e.g. `"SPY"`)
- `expiration` - Expiration date as `YYYYMMDD` string
- `strike` - Strike price in dollars as a string (e.g. `"500"` or `"17.5"`)
- `right` - `"C"` for call, `"P"` for put

### option_list_symbols

List all available option underlying symbols.

::: code-group
```rust [Rust]
let symbols = tdx.option_list_symbols().await?;
```
```python [Python]
symbols = tdx.option_list_symbols()
```
```go [Go]
symbols, err := client.OptionListSymbols()
```
```cpp [C++]
auto symbols = client.option_list_symbols();
```
:::

**Parameters:** None

**Returns:** List of underlying symbol strings.

---

### option_list_dates

List available dates for an option contract by request type.

::: code-group
```rust [Rust]
let dates = tdx.option_list_dates("TRADE", "SPY", "20241220", "500", "C").await?;
```
```python [Python]
dates = tdx.option_list_dates("TRADE", "SPY", "20241220", "500", "C")
```
```go [Go]
dates, err := client.OptionListDates("TRADE", "SPY", "20241220", "500", "C")
```
```cpp [C++]
auto dates = client.option_list_dates("TRADE", "SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type: `"TRADE"`, `"QUOTE"`, etc. |
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |

**Returns:** List of date strings (`YYYYMMDD`).

---

### option_list_expirations

List all expiration dates for an underlying.

::: code-group
```rust [Rust]
let exps = tdx.option_list_expirations("SPY").await?;
```
```python [Python]
exps = tdx.option_list_expirations("SPY")
```
```go [Go]
exps, err := client.OptionListExpirations("SPY")
```
```cpp [C++]
auto exps = client.option_list_expirations("SPY");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |

**Returns:** List of expiration date strings (`YYYYMMDD`).

---

### option_list_strikes

List strike prices for a given expiration.

::: code-group
```rust [Rust]
let strikes = tdx.option_list_strikes("SPY", "20241220").await?;
```
```python [Python]
strikes = tdx.option_list_strikes("SPY", "20241220")
```
```go [Go]
strikes, err := client.OptionListStrikes("SPY", "20241220")
```
```cpp [C++]
auto strikes = client.option_list_strikes("SPY", "20241220");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |

**Returns:** List of strike price strings in dollars.

---

### option_list_contracts

List all option contracts for a symbol on a given date.

::: code-group
```rust [Rust]
let contracts = tdx.option_list_contracts("TRADE", "SPY", "20240315").await?;
```
```python [Python]
contracts = tdx.option_list_contracts("TRADE", "SPY", "20240315")
```
```go [Go]
contracts, err := client.OptionListContracts("TRADE", "SPY", "20240315")
```
```cpp [C++]
auto contracts = client.option_list_contracts("TRADE", "SPY", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type |
| `symbol` | string | Yes | Underlying symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `max_dte` | int | No | Maximum days to expiration filter |

**Returns:** Array of OptionContract records with root, expiration, strike, right.

---

### option_snapshot_ohlc

Latest OHLC snapshot for an option contract.

::: code-group
```rust [Rust]
let bars = tdx.option_snapshot_ohlc("SPY", "20241220", "500", "C").await?;
```
```python [Python]
bars = tdx.option_snapshot_ohlc("SPY", "20241220", "500", "C")
```
```go [Go]
bars, err := client.OptionSnapshotOHLC("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto bars = client.option_snapshot_ohlc("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [OhlcTick](#ohlctick).

---

### option_snapshot_trade

Latest trade snapshot for an option contract.

::: code-group
```rust [Rust]
let trades = tdx.option_snapshot_trade("SPY", "20241220", "500", "C").await?;
```
```python [Python]
trades = tdx.option_snapshot_trade("SPY", "20241220", "500", "C")
```
```go [Go]
trades, err := client.OptionSnapshotTrade("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto trades = client.option_snapshot_trade("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [TradeTick](#tradetick).

---

### option_snapshot_quote

Latest NBBO quote snapshot for an option contract.

::: code-group
```rust [Rust]
let quotes = tdx.option_snapshot_quote("SPY", "20241220", "500", "C").await?;
```
```python [Python]
quotes = tdx.option_snapshot_quote("SPY", "20241220", "500", "C")
```
```go [Go]
quotes, err := client.OptionSnapshotQuote("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto quotes = client.option_snapshot_quote("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [QuoteTick](#quotetick).

---

### option_snapshot_open_interest

Latest open interest snapshot for an option contract.

::: code-group
```rust [Rust]
let oi = tdx.option_snapshot_open_interest("SPY", "20241220", "500", "C").await?;
```
```python [Python]
oi = tdx.option_snapshot_open_interest("SPY", "20241220", "500", "C")
```
```go [Go]
oi, err := client.OptionSnapshotOpenInterest("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto oi = client.option_snapshot_open_interest("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** Array of OpenInterestTick records with ms_of_day, open_interest, date.

---

### option_snapshot_market_value

Latest market value snapshot for an option contract.

::: code-group
```rust [Rust]
let mv = tdx.option_snapshot_market_value("SPY", "20241220", "500", "C").await?;
```
```python [Python]
mv = tdx.option_snapshot_market_value("SPY", "20241220", "500", "C")
```
```go [Go]
mv, err := client.OptionSnapshotMarketValue("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto mv = client.option_snapshot_market_value("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** Array of MarketValueTick records with market cap, shares outstanding, enterprise value, book value, free float.

---

### option_snapshot_greeks_implied_volatility

Implied volatility snapshot for an option contract.

::: code-group
```rust [Rust]
let iv = tdx.option_snapshot_greeks_implied_volatility("SPY", "20241220", "500", "C").await?;
```
```python [Python]
iv = tdx.option_snapshot_greeks_implied_volatility("SPY", "20241220", "500", "C")
```
```go [Go]
iv, err := client.OptionSnapshotGreeksIV("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto iv = client.option_snapshot_greeks_implied_volatility("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type (e.g. `"SOFR"`) |
| `rate_value` | float | No | Override interest rate value |
| `stock_price` | float | No | Override underlying price |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |
| `use_market_value` | bool | No | Use market value instead of last trade |

**Returns:** Array of IvTick records with implied_volatility, iv_error.

**Tier:** Pro

---

### option_snapshot_greeks_all

Snapshot of all Greeks for an option contract.

::: code-group
```rust [Rust]
let greeks = tdx.option_snapshot_greeks_all("SPY", "20241220", "500", "C").await?;
```
```python [Python]
greeks = tdx.option_snapshot_greeks_all("SPY", "20241220", "500", "C")
```
```go [Go]
greeks, err := client.OptionSnapshotGreeksAll("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto greeks = client.option_snapshot_greeks_all("SPY", "20241220", "500", "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `stock_price` | float | No | Override underlying price |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day |
| `use_market_value` | bool | No | Use market value instead of last trade |

**Returns:** Array of GreeksTick records with all 22 Greeks.

**Tier:** Pro

---

### option_snapshot_greeks_first_order

First-order Greeks snapshot (delta, theta, vega, rho, epsilon, lambda).

::: code-group
```rust [Rust]
let g = tdx.option_snapshot_greeks_first_order("SPY", "20241220", "500", "C").await?;
```
```python [Python]
g = tdx.option_snapshot_greeks_first_order("SPY", "20241220", "500", "C")
```
```go [Go]
g, err := client.OptionSnapshotGreeksFirstOrder("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto g = client.option_snapshot_greeks_first_order("SPY", "20241220", "500", "C");
```
:::

Parameters are identical to [option_snapshot_greeks_all](#option_snapshot_greeks_all).

**Returns:** Array of GreeksTick records with first-order Greeks (delta, theta, vega, rho).

**Tier:** Pro

---

### option_snapshot_greeks_second_order

Second-order Greeks snapshot (gamma, vanna, charm, vomma, veta).

::: code-group
```rust [Rust]
let g = tdx.option_snapshot_greeks_second_order("SPY", "20241220", "500", "C").await?;
```
```python [Python]
g = tdx.option_snapshot_greeks_second_order("SPY", "20241220", "500", "C")
```
```go [Go]
g, err := client.OptionSnapshotGreeksSecondOrder("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto g = client.option_snapshot_greeks_second_order("SPY", "20241220", "500", "C");
```
:::

Parameters are identical to [option_snapshot_greeks_all](#option_snapshot_greeks_all).

**Returns:** Array of GreeksTick records with second-order Greeks (gamma, vanna, charm, vomma).

**Tier:** Pro

---

### option_snapshot_greeks_third_order

Third-order Greeks snapshot (speed, zomma, color, ultima).

::: code-group
```rust [Rust]
let g = tdx.option_snapshot_greeks_third_order("SPY", "20241220", "500", "C").await?;
```
```python [Python]
g = tdx.option_snapshot_greeks_third_order("SPY", "20241220", "500", "C")
```
```go [Go]
g, err := client.OptionSnapshotGreeksThirdOrder("SPY", "20241220", "500", "C")
```
```cpp [C++]
auto g = client.option_snapshot_greeks_third_order("SPY", "20241220", "500", "C");
```
:::

Parameters are identical to [option_snapshot_greeks_all](#option_snapshot_greeks_all).

**Returns:** Array of GreeksTick records with third-order Greeks (speed, zomma, color, ultima).

**Tier:** Pro

---

### option_history_eod

End-of-day option data across a date range.

::: code-group
```rust [Rust]
let eod = tdx.option_history_eod(
    "SPY", "20241220", "500", "C", "20240101", "20240301"
).await?;
```
```python [Python]
eod = tdx.option_history_eod("SPY", "20241220", "500", "C", "20240101", "20240301")
```
```go [Go]
eod, err := client.OptionHistoryEOD("SPY", "20241220", "500", "C", "20240101", "20240301")
```
```cpp [C++]
auto eod = client.option_history_eod("SPY", "20241220", "500", "C", "20240101", "20240301");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [EodTick](#eodtick).

---

### option_history_ohlc

Intraday OHLC bars for an option contract.

::: code-group
```rust [Rust]
let bars = tdx.option_history_ohlc(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
bars = tdx.option_history_ohlc("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
bars, err := client.OptionHistoryOHLC("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto bars = client.option_history_ohlc("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [OhlcTick](#ohlctick).

---

### option_history_trade

All trades for an option contract on a given date.

::: code-group
```rust [Rust]
let trades = tdx.option_history_trade("SPY", "20241220", "500", "C", "20240315").await?;
```
```python [Python]
trades = tdx.option_history_trade("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
trades, err := client.OptionHistoryTrade("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto trades = client.option_history_trade("SPY", "20241220", "500", "C", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [TradeTick](#tradetick).

**Tier:** Standard+

---

### option_history_quote

NBBO quotes for an option contract.

::: code-group
```rust [Rust]
let quotes = tdx.option_history_quote(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
quotes = tdx.option_history_quote("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
quotes, err := client.OptionHistoryQuote("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto quotes = client.option_history_quote("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [QuoteTick](#quotetick).

**Tier:** Standard+

---

### option_history_trade_quote

Combined trade + quote ticks for an option contract.

::: code-group
```rust [Rust]
let tq = tdx.option_history_trade_quote("SPY", "20241220", "500", "C", "20240315").await?;
```
```python [Python]
tq = tdx.option_history_trade_quote("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
tq, err := client.OptionHistoryTradeQuote("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto tq = client.option_history_trade_quote("SPY", "20241220", "500", "C", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `exclusive` | bool | No | Exclusive time bounds |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** [TradeQuoteTick](#tradequotetick) data.

**Tier:** Pro

---

### option_history_open_interest

Open interest history for an option contract.

::: code-group
```rust [Rust]
let oi = tdx.option_history_open_interest("SPY", "20241220", "500", "C", "20240315").await?;
```
```python [Python]
oi = tdx.option_history_open_interest("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
oi, err := client.OptionHistoryOpenInterest("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto oi = client.option_history_open_interest("SPY", "20241220", "500", "C", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** Array of OpenInterestTick records with ms_of_day, open_interest, date.

---

### option_history_greeks_eod

End-of-day Greeks history for an option contract.

::: code-group
```rust [Rust]
let g = tdx.option_history_greeks_eod(
    "SPY", "20241220", "500", "C", "20240101", "20240301"
).await?;
```
```python [Python]
g = tdx.option_history_greeks_eod("SPY", "20241220", "500", "C", "20240101", "20240301")
```
```go [Go]
g, err := client.OptionHistoryGreeksEOD("SPY", "20241220", "500", "C", "20240101", "20240301")
```
```cpp [C++]
auto g = client.option_history_greeks_eod("SPY", "20241220", "500", "C", "20240101", "20240301");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `underlyer_use_nbbo` | bool | No | Use NBBO for underlying price |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** Array of GreeksTick records with EOD Greeks per date.

**Tier:** Pro

---

### option_history_greeks_all

All Greeks history at a given interval (intraday).

::: code-group
```rust [Rust]
let g = tdx.option_history_greeks_all(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = tdx.option_history_greeks_all("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksAll("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_all("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `strike_range` | int | No | Strike range filter |

**Returns:** Array of GreeksTick records with all 22 Greeks at each sampled point.

**Tier:** Pro

---

### option_history_trade_greeks_all

All Greeks computed on each individual trade.

::: code-group
```rust [Rust]
let g = tdx.option_history_trade_greeks_all("SPY", "20241220", "500", "C", "20240315").await?;
```
```python [Python]
g = tdx.option_history_trade_greeks_all("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
g, err := client.OptionHistoryTradeGreeksAll("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto g = client.option_history_trade_greeks_all("SPY", "20241220", "500", "C", "20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** Array of GreeksTick records with all 22 Greeks per trade.

**Tier:** Pro

---

### option_history_greeks_first_order

First-order Greeks history at a given interval.

::: code-group
```rust [Rust]
let g = tdx.option_history_greeks_first_order(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = tdx.option_history_greeks_first_order("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksFirstOrder("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_first_order("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

Parameters are identical to [option_history_greeks_all](#option_history_greeks_all).

**Returns:** Array of GreeksTick records with first-order Greeks at each sampled point.

**Tier:** Pro

---

### option_history_trade_greeks_first_order

First-order Greeks computed on each individual trade.

::: code-group
```rust [Rust]
let g = tdx.option_history_trade_greeks_first_order(
    "SPY", "20241220", "500", "C", "20240315"
).await?;
```
```python [Python]
g = tdx.option_history_trade_greeks_first_order("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
g, err := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto g = client.option_history_trade_greeks_first_order("SPY", "20241220", "500", "C", "20240315");
```
:::

Parameters are identical to [option_history_trade_greeks_all](#option_history_trade_greeks_all).

**Returns:** Array of GreeksTick records with first-order Greeks per trade.

**Tier:** Pro

---

### option_history_greeks_second_order

Second-order Greeks history at a given interval.

::: code-group
```rust [Rust]
let g = tdx.option_history_greeks_second_order(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = tdx.option_history_greeks_second_order("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksSecondOrder("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_second_order("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

Parameters are identical to [option_history_greeks_all](#option_history_greeks_all).

**Returns:** Array of GreeksTick records with second-order Greeks at each sampled point.

**Tier:** Pro

---

### option_history_trade_greeks_second_order

Second-order Greeks computed on each individual trade.

::: code-group
```rust [Rust]
let g = tdx.option_history_trade_greeks_second_order(
    "SPY", "20241220", "500", "C", "20240315"
).await?;
```
```python [Python]
g = tdx.option_history_trade_greeks_second_order("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
g, err := client.OptionHistoryTradeGreeksSecondOrder("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto g = client.option_history_trade_greeks_second_order("SPY", "20241220", "500", "C", "20240315");
```
:::

Parameters are identical to [option_history_trade_greeks_all](#option_history_trade_greeks_all).

**Returns:** Array of GreeksTick records with second-order Greeks per trade.

**Tier:** Pro

---

### option_history_greeks_third_order

Third-order Greeks history at a given interval.

::: code-group
```rust [Rust]
let g = tdx.option_history_greeks_third_order(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = tdx.option_history_greeks_third_order("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksThirdOrder("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_third_order("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

Parameters are identical to [option_history_greeks_all](#option_history_greeks_all).

**Returns:** Array of GreeksTick records with third-order Greeks at each sampled point.

**Tier:** Pro

---

### option_history_trade_greeks_third_order

Third-order Greeks computed on each individual trade.

::: code-group
```rust [Rust]
let g = tdx.option_history_trade_greeks_third_order(
    "SPY", "20241220", "500", "C", "20240315"
).await?;
```
```python [Python]
g = tdx.option_history_trade_greeks_third_order("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
g, err := client.OptionHistoryTradeGreeksThirdOrder("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto g = client.option_history_trade_greeks_third_order("SPY", "20241220", "500", "C", "20240315");
```
:::

Parameters are identical to [option_history_trade_greeks_all](#option_history_trade_greeks_all).

**Returns:** Array of GreeksTick records with third-order Greeks per trade.

**Tier:** Pro

---

### option_history_greeks_implied_volatility

Implied volatility history at a given interval.

::: code-group
```rust [Rust]
let iv = tdx.option_history_greeks_implied_volatility(
    "SPY", "20241220", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
iv = tdx.option_history_greeks_implied_volatility("SPY", "20241220", "500", "C", "20240315", "60000")
```
```go [Go]
iv, err := client.OptionHistoryGreeksImpliedVolatility("SPY", "20241220", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto iv = client.option_history_greeks_implied_volatility("SPY", "20241220", "500", "C", "20240315", "60000");
```
:::

Parameters are identical to [option_history_greeks_all](#option_history_greeks_all).

**Returns:** Array of IvTick records with implied volatility at each sampled point.

**Tier:** Pro

---

### option_history_trade_greeks_implied_volatility

Implied volatility computed on each individual trade.

::: code-group
```rust [Rust]
let iv = tdx.option_history_trade_greeks_implied_volatility(
    "SPY", "20241220", "500", "C", "20240315"
).await?;
```
```python [Python]
iv = tdx.option_history_trade_greeks_implied_volatility("SPY", "20241220", "500", "C", "20240315")
```
```go [Go]
iv, err := client.OptionHistoryTradeGreeksImpliedVolatility("SPY", "20241220", "500", "C", "20240315")
```
```cpp [C++]
auto iv = client.option_history_trade_greeks_implied_volatility("SPY", "20241220", "500", "C", "20240315");
```
:::

Parameters are identical to [option_history_trade_greeks_all](#option_history_trade_greeks_all).

**Returns:** Array of IvTick records with IV per trade.

**Tier:** Pro

---

### option_at_time_trade

Trade at a specific time of day across a date range for an option contract.

::: code-group
```rust [Rust]
let trades = tdx.option_at_time_trade(
    "SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000"
).await?;
```
```python [Python]
trades = tdx.option_at_time_trade("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000")
```
```go [Go]
trades, err := client.OptionAtTimeTrade("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000")
```
```cpp [C++]
auto trades = client.option_at_time_trade("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [TradeTick](#tradetick), one per date.

---

### option_at_time_quote

Quote at a specific time of day across a date range for an option contract.

::: code-group
```rust [Rust]
let quotes = tdx.option_at_time_quote(
    "SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000"
).await?;
```
```python [Python]
quotes = tdx.option_at_time_quote("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000")
```
```go [Go]
quotes, err := client.OptionAtTimeQuote("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000")
```
```cpp [C++]
auto quotes = client.option_at_time_quote("SPY", "20241220", "500", "C", "20240101", "20240301", "09:30:00.000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date |
| `strike` | string | Yes | Strike price in dollars as a string |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

**Returns:** List of [QuoteTick](#quotetick), one per date.

---

## Index Endpoints

### index_list_symbols

List all available index symbols.

::: code-group
```rust [Rust]
let symbols = tdx.index_list_symbols().await?;
```
```python [Python]
symbols = tdx.index_list_symbols()
```
```go [Go]
symbols, err := client.IndexListSymbols()
```
```cpp [C++]
auto symbols = client.index_list_symbols();
```
:::

**Parameters:** None

**Returns:** List of index symbol strings (e.g. `"SPX"`, `"VIX"`, `"DJI"`).

---

### index_list_dates

List available dates for an index symbol.

::: code-group
```rust [Rust]
let dates = tdx.index_list_dates("SPX").await?;
```
```python [Python]
dates = tdx.index_list_dates("SPX")
```
```go [Go]
dates, err := client.IndexListDates("SPX")
```
```cpp [C++]
auto dates = client.index_list_dates("SPX");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol |

**Returns:** List of date strings (`YYYYMMDD`).

---

### index_snapshot_ohlc

Latest OHLC snapshot for one or more indices.

::: code-group
```rust [Rust]
let bars = tdx.index_snapshot_ohlc(&["SPX", "VIX"]).await?;
```
```python [Python]
bars = tdx.index_snapshot_ohlc(["SPX", "VIX"])
```
```go [Go]
bars, err := client.IndexSnapshotOHLC([]string{"SPX", "VIX"})
```
```cpp [C++]
auto bars = client.index_snapshot_ohlc({"SPX", "VIX"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** List of [OhlcTick](#ohlctick).

---

### index_snapshot_price

Latest price snapshot for one or more indices.

::: code-group
```rust [Rust]
let prices = tdx.index_snapshot_price(&["SPX"]).await?;
```
```python [Python]
prices = tdx.index_snapshot_price(["SPX"])
```
```go [Go]
prices, err := client.IndexSnapshotPrice([]string{"SPX"})
```
```cpp [C++]
auto prices = client.index_snapshot_price({"SPX"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** Array of PriceTick records with ms_of_day, price, date.

---

### index_snapshot_market_value

Latest market value snapshot for one or more indices.

::: code-group
```rust [Rust]
let mv = tdx.index_snapshot_market_value(&["SPX"]).await?;
```
```python [Python]
mv = tdx.index_snapshot_market_value(["SPX"])
```
```go [Go]
mv, err := client.IndexSnapshotMarketValue([]string{"SPX"})
```
```cpp [C++]
auto mv = client.index_snapshot_market_value({"SPX"});
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

**Returns:** Array of MarketValueTick records with market cap, shares outstanding, enterprise value, book value, free float.

---

### index_history_eod

End-of-day index data across a date range.

::: code-group
```rust [Rust]
let eod = tdx.index_history_eod("SPX", "20240101", "20240301").await?;
```
```python [Python]
eod = tdx.index_history_eod("SPX", "20240101", "20240301")
```
```go [Go]
eod, err := client.IndexHistoryEOD("SPX", "20240101", "20240301")
```
```cpp [C++]
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

**Returns:** List of [EodTick](#eodtick).

---

### index_history_ohlc

Intraday OHLC bars for an index across a date range.

::: code-group
```rust [Rust]
let bars = tdx.index_history_ohlc("SPX", "20240101", "20240301", "60000").await?;
```
```python [Python]
bars = tdx.index_history_ohlc("SPX", "20240101", "20240301", "60000")
```
```go [Go]
bars, err := client.IndexHistoryOHLC("SPX", "20240101", "20240301", "60000")
```
```cpp [C++]
auto bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |

**Returns:** List of [OhlcTick](#ohlctick).

---

### index_history_price

Intraday price history for an index.

::: code-group
```rust [Rust]
let prices = tdx.index_history_price("SPX", "20240315", "60000").await?;
```
```python [Python]
prices = tdx.index_history_price("SPX", "20240315", "60000")
```
```go [Go]
prices, err := client.IndexHistoryPrice("SPX", "20240315", "60000")
```
```cpp [C++]
auto prices = client.index_history_price("SPX", "20240315", "60000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Accepts milliseconds (`"60000"`) or shorthand (`"1m"`). Valid presets: `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |

**Returns:** Array of PriceTick records with price at each sampled point.

---

### index_at_time_price

Index price at a specific time of day across a date range.

::: code-group
```rust [Rust]
let prices = tdx.index_at_time_price("SPX", "20240101", "20240301", "09:30:00.000").await?;
```
```python [Python]
prices = tdx.index_at_time_price("SPX", "20240101", "20240301", "09:30:00.000")
```
```go [Go]
prices, err := client.IndexAtTimePrice("SPX", "20240101", "20240301", "09:30:00.000")
```
```cpp [C++]
auto prices = client.index_at_time_price("SPX", "20240101", "20240301", "09:30:00.000");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS` |

**Returns:** Array of PriceTick records with one price per date.

---

## Calendar Endpoints

### calendar_open_today

Check whether the market is open today.

::: code-group
```rust [Rust]
let info = tdx.calendar_open_today().await?;
```
```python [Python]
info = tdx.calendar_open_today()
```
```go [Go]
info, err := client.CalendarOpenToday()
```
```cpp [C++]
auto info = client.calendar_open_today();
```
:::

**Parameters:** None

**Returns:** Array of CalendarDay records with is_open, open_time, close_time.

---

### calendar_on_date

Calendar information for a specific date.

::: code-group
```rust [Rust]
let info = tdx.calendar_on_date("20240315").await?;
```
```python [Python]
info = tdx.calendar_on_date("20240315")
```
```go [Go]
info, err := client.CalendarOnDate("20240315")
```
```cpp [C++]
auto info = client.calendar_on_date("20240315");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `date` | string | Yes | Date (`YYYYMMDD`) |

**Returns:** Array of CalendarDay records with calendar info for the date.

---

### calendar_year

Calendar information for an entire year.

::: code-group
```rust [Rust]
let cal = tdx.calendar_year("2024").await?;
```
```python [Python]
cal = tdx.calendar_year("2024")
```
```go [Go]
cal, err := client.CalendarYear("2024")
```
```cpp [C++]
auto cal = client.calendar_year("2024");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `year` | string | Yes | 4-digit year (e.g. `"2024"`) |

**Returns:** Array of CalendarDay records with calendar info for every trading day.

---

## Interest Rate Endpoints

### interest_rate_history_eod

End-of-day interest rate history.

::: code-group
```rust [Rust]
let rates = tdx.interest_rate_history_eod("SOFR", "20240101", "20240301").await?;
```
```python [Python]
rates = tdx.interest_rate_history_eod("SOFR", "20240101", "20240301")
```
```go [Go]
rates, err := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
```
```cpp [C++]
auto rates = client.interest_rate_history_eod("SOFR", "20240101", "20240301");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Rate symbol (e.g. `"SOFR"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

**Returns:** Array of InterestRateTick records with rate per date.

---

## Greeks Calculator

Full Black-Scholes calculator with 20 individual Greek functions, an IV solver, and a combined `all_greeks` function. Computed locally - no server round-trip.

### all_greeks

Compute all 22 Greeks at once. Solves for IV first, then computes all Greeks using the solved volatility. This is much more efficient than calling individual functions because it avoids redundant `d1`/`d2`/`exp()`/`norm_cdf()` recomputation.

::: code-group
```rust [Rust]
use thetadatadx::all_greeks;

let g = all_greeks(
    450.0,          // spot
    455.0,          // strike
    0.05,           // risk-free rate
    0.015,          // dividend yield
    30.0 / 365.0,   // time to expiration (years)
    8.50,           // market price of option
    "C",            // right ("C"/"P" or "call"/"put", case-insensitive)
);
println!("IV: {:.4}, Delta: {:.4}", g.iv, g.delta);
```
```python [Python]
from thetadatadx import all_greeks

g = all_greeks(450.0, 455.0, 0.05, 0.015, 30.0 / 365.0, 8.50, "C")
print(f"IV: {g['iv']:.4f}, Delta: {g['delta']:.4f}")
```
```go [Go]
g, err := thetadatadx.AllGreeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
fmt.Printf("IV: %.4f, Delta: %.4f\n", g.IV, g.Delta)
```
```cpp [C++]
auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0 / 365.0, 8.50, "C");
std::cout << "IV: " << g.iv << ", Delta: " << g.delta << std::endl;
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `spot` | float | Yes | Underlying (spot) price |
| `strike` | float | Yes | Strike price |
| `rate` | float | Yes | Risk-free interest rate (annualized) |
| `div_yield` | float | Yes | Continuous dividend yield (annualized) |
| `tte` | float | Yes | Time to expiration in years |
| `option_price` | float | Yes | Market price of the option |
| `right` | string | Yes | Option side: `"C"`/`"P"` or `"call"`/`"put"` (case-insensitive) |

**Returns:** [GreeksResult](#greeksresult) containing all 22 fields.

---

### implied_volatility

Solve for implied volatility using bisection (up to 128 iterations).

::: code-group
```rust [Rust]
use thetadatadx::implied_volatility;

let (iv, err) = implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
```
```python [Python]
from thetadatadx import implied_volatility

iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
```
```go [Go]
iv, ivErr, err := thetadatadx.ImpliedVolatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
```
```cpp [C++]
auto [iv, err] = tdx::implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
```
:::

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `spot` | float | Yes | Underlying price |
| `strike` | float | Yes | Strike price |
| `rate` | float | Yes | Risk-free interest rate |
| `div_yield` | float | Yes | Dividend yield |
| `tte` | float | Yes | Time to expiration in years |
| `option_price` | float | Yes | Market price of the option |
| `right` | string | Yes | Option side: `"C"`/`"P"` or `"call"`/`"put"` (case-insensitive) |

**Returns:** Tuple of `(iv, error)` where `error` is the relative difference `(theoretical - market) / market`.

---

### Individual Greek Functions

All individual functions share these parameters. Not all functions take `right` - symmetric Greeks omit it.

| Parameter | Type | Description |
|-----------|------|-------------|
| `s` | float | Spot price |
| `x` | float | Strike price |
| `v` | float | Volatility (sigma) |
| `r` | float | Risk-free rate |
| `q` | float | Dividend yield |
| `t` | float | Time to expiration (years) |
| `right` | str | `"C"` / `"call"` / `"P"` / `"put"` (case-insensitive) - only for directional Greeks |

#### First-Order Greeks

| Function | Signature | Description |
|----------|-----------|-------------|
| `value` | `(s, x, v, r, q, t, right) -> f64` | Black-Scholes theoretical option value |
| `delta` | `(s, x, v, r, q, t, right) -> f64` | Rate of change of value w.r.t. spot price |
| `theta` | `(s, x, v, r, q, t, right) -> f64` | Time decay (daily, divided by 365) |
| `vega` | `(s, x, v, r, q, t) -> f64` | Sensitivity to volatility |
| `rho` | `(s, x, v, r, q, t, right) -> f64` | Sensitivity to interest rate |
| `epsilon` | `(s, x, v, r, q, t, right) -> f64` | Sensitivity to dividend yield |
| `lambda` | `(s, x, v, r, q, t, right) -> f64` | Leverage ratio (elasticity) |

#### Second-Order Greeks

| Function | Signature | Description |
|----------|-----------|-------------|
| `gamma` | `(s, x, v, r, q, t) -> f64` | Rate of change of delta w.r.t. spot |
| `vanna` | `(s, x, v, r, q, t) -> f64` | Cross-sensitivity of delta to volatility |
| `charm` | `(s, x, v, r, q, t, right) -> f64` | Rate of change of delta w.r.t. time (delta decay) |
| `vomma` | `(s, x, v, r, q, t) -> f64` | Rate of change of vega w.r.t. volatility |
| `veta` | `(s, x, v, r, q, t) -> f64` | Rate of change of vega w.r.t. time |

#### Third-Order Greeks

| Function | Signature | Description |
|----------|-----------|-------------|
| `speed` | `(s, x, v, r, q, t) -> f64` | Rate of change of gamma w.r.t. spot |
| `zomma` | `(s, x, v, r, q, t) -> f64` | Rate of change of gamma w.r.t. volatility |
| `color` | `(s, x, v, r, q, t) -> f64` | Rate of change of gamma w.r.t. time |
| `ultima` | `(s, x, v, r, q, t) -> f64` | Rate of change of vomma w.r.t. volatility (clamped to [-100, 100]) |

#### Auxiliary

| Function | Signature | Description |
|----------|-----------|-------------|
| `dual_delta` | `(s, x, v, r, q, t, right) -> f64` | Sensitivity of value w.r.t. strike |
| `dual_gamma` | `(s, x, v, r, q, t) -> f64` | Second derivative w.r.t. strike |
| `d1` | `(s, x, v, r, q, t) -> f64` | Black-Scholes d1 term |
| `d2` | `(s, x, v, r, q, t) -> f64` | Black-Scholes d2 term |

---

## Streaming (FPSS)

Real-time market data streaming via FPSS (Fast Protocol Streaming Service) over TLS/TCP. The streaming connection is established lazily and managed through the main client in Rust and Python; Go and C++ use a dedicated `FpssClient`.

### Starting the Stream

::: code-group
```rust [Rust]
tdx.start_streaming(|event: &FpssEvent| {
    match event {
        FpssEvent::Data(data) => println!("{:?}", data),
        FpssEvent::Control(ctrl) => println!("{:?}", ctrl),
        _ => {}
    }
})?;
```
```python [Python]
tdx.start_streaming()
```
```go [Go]
fpss := thetadatadx.NewFpssClient(creds, config)
defer fpss.Close()
```
```cpp [C++]
tdx::FpssClient fpss(creds, tdx::Config::production());
```
:::

### Subscribing

::: code-group
```rust [Rust]
let req_id = tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
let req_id = tdx.subscribe_trades(&Contract::stock("AAPL"))?;
let req_id = tdx.subscribe_open_interest(&Contract::stock("AAPL"))?;
let req_id = tdx.subscribe_full_trades(SecType::Stock)?;
```
```python [Python]
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("AAPL")
```
```go [Go]
reqID, err := fpss.SubscribeQuotes("AAPL")
reqID, err := fpss.SubscribeTrades("AAPL")
reqID, err := fpss.SubscribeOpenInterest("AAPL")
reqID, err := fpss.SubscribeFullTrades("STOCK")
```
```cpp [C++]
int req_id = fpss.subscribe_quotes("AAPL");
int req_id = fpss.subscribe_trades("AAPL");
int req_id = fpss.subscribe_open_interest("AAPL");
int req_id = fpss.subscribe_full_trades("STOCK");
```
:::

| Method | Description |
|--------|-------------|
| `subscribe_quotes` | Subscribe to real-time NBBO quote updates |
| `subscribe_trades` | Subscribe to real-time trade updates |
| `subscribe_open_interest` | Subscribe to open interest updates |
| `subscribe_full_trades` | Subscribe to all trades for a security type |
| `subscribe_full_open_interest` | Subscribe to all OI for a security type |
| `unsubscribe_full_trades` | Unsubscribe from all trades for a security type |
| `unsubscribe_full_open_interest` | Unsubscribe from all OI for a security type |

All subscription methods return a request ID. The server confirms via a `ReqResponse` control event.

### Unsubscribing

::: code-group
```rust [Rust]
tdx.unsubscribe_quotes(&Contract::stock("AAPL"))?;
tdx.unsubscribe_trades(&Contract::stock("AAPL"))?;
tdx.unsubscribe_open_interest(&Contract::stock("AAPL"))?;
```
```python [Python]
# Not exposed in Python - use stop_streaming()
```
```go [Go]
fpss.UnsubscribeQuotes("AAPL")
fpss.UnsubscribeTrades("AAPL")
fpss.UnsubscribeOpenInterest("AAPL")
```
```cpp [C++]
fpss.unsubscribe_quotes("AAPL");
fpss.unsubscribe_trades("AAPL");
fpss.unsubscribe_open_interest("AAPL");
```
:::

### Receiving Events

::: code-group
```rust [Rust]
// Events arrive via the callback passed to start_streaming()
tdx.start_streaming(|event| {
    if let FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) = event {
        println!("Trade: contract={}, price={}, size={}", contract_id, price, size);
    }
})?;
```
```python [Python]
event = tdx.next_event(timeout_ms=5000)  # returns dict or None
if event:
    print(event)
```
```go [Go]
event, err := fpss.NextEvent(5000)  // returns *FpssEvent or nil
```
```cpp [C++]
FpssEventPtr event = fpss.next_event(5000);  // nullptr on timeout
```
:::

### Shutting Down

::: code-group
```rust [Rust]
tdx.stop_streaming();
```
```python [Python]
tdx.stop_streaming()
```
```go [Go]
fpss.Shutdown()
```
```cpp [C++]
fpss.shutdown();
```
:::

### Streaming State

| Method | Returns | SDK availability | Description |
|--------|---------|------------------|-------------|
| `is_streaming` | bool | Rust/Python only | Check if the unified streaming connection is live |
| `contract_map` | `HashMap<i32, Contract>` (Rust), `dict[int, Contract]` (Python), `map[int32]string` (Go), `map<int32_t, string>` (C++) | All SDKs | Get full contract ID mapping |
| `contract_lookup` | string/optional | All SDKs (FFI-based, returns NULL/"" for not-found) | Look up a single contract by server-assigned ID |
| `active_subscriptions` | list/typed structs | All SDKs | Get list of active subscriptions |
| `subscribe_option_*` / `unsubscribe_option_*` | int | All SDKs | Option-level subscribe/unsubscribe by `(symbol, expiration, strike, right)` |
| `reconnect` | void / result | All SDKs | Reconnect streaming and re-subscribe the previous subscription set |

### FpssEvent Types

Events are delivered as one of three categories:

**FpssData** - Market data events (all variants include `received_at_ns: u64` wall-clock timestamp):
- `Quote` - NBBO quote update (bid, ask, sizes, exchanges, conditions)
- `Trade` - Trade execution (price, size, exchange, conditions)
- `OpenInterest` - Open interest update
- `Ohlcvc` - Aggregated OHLCVC bar (derived from trades via internal accumulator; `volume`/`count` are `i64` to avoid overflow on high-volume symbols)

**FpssControl** - Lifecycle events:
- `LoginSuccess` - Authentication successful (includes permissions string)
- `ContractAssigned` - Server assigned an ID to a contract
- `ReqResponse` - Server confirmed a subscription request
- `MarketOpen` / `MarketClose` - Market state transitions
- `ServerError` / `Error` - Error conditions
- `Disconnected` - Connection lost (includes reason code)

**RawData** - Unparsed frames with unknown message codes.

### Reconnection

The `reconnect_delay` function returns the appropriate wait time based on the disconnect reason:

- Returns `None` (no reconnect) for permanent errors: invalid credentials, account already connected, free account
- Returns `130,000 ms` for rate limiting (`TooManyRequests`)
- Returns `2,000 ms` for all other transient errors

---

## Streaming Endpoint Variants

Streaming variants (`*_stream`) use per-chunk callbacks and are available in the Rust SDK only. Other languages use the non-streaming equivalents which return complete result sets.

### stock_history_trade_stream

```rust
tdx.stock_history_trade_stream("AAPL", "20240315")
    .stream(|trades: &[TradeTick]| {
        for t in trades {
            println!("{}: {}", t.date, t.price);
        }
    }).await?;
```

### stock_history_quote_stream

```rust
tdx.stock_history_quote_stream("AAPL", "20240315", "0")
    .stream(|quotes: &[QuoteTick]| {
    println!("Chunk: {} quotes", quotes.len());
}).await?;
```

### option_history_trade_stream

```rust
tdx.option_history_trade_stream("SPY", "20241220", "500", "C", "20240315")
    .stream(|trades: &[TradeTick]| {
        println!("Chunk: {} trades", trades.len());
    }).await?;
```

### option_history_quote_stream

```rust
tdx.option_history_quote_stream("SPY", "20241220", "500", "C", "20240315", "0")
    .stream(|quotes: &[QuoteTick]| {
        println!("Chunk: {} quotes", quotes.len());
    }).await?;
```

---

## Types and Enums

### Contract Identification Fields

10 tick types carry contract identification fields populated by the server on wildcard queries (pass `0` for expiration/strike). On single-contract queries these fields are `0`/empty.

| Field | Type (Rust/FFI) | Type (Go) | Description |
|-------|-----------------|-----------|-------------|
| `expiration` | i32 | int32 | Contract expiration (YYYYMMDD). 0 if absent. |
| `strike` | f64 | float64 | Strike price (decoded to f64). |
| `right` | i32 | string | Contract right. Rust/FFI: 67=Call, 80=Put. Go: `"C"`, `"P"`, `""`. |

Helper methods (all 10 types): `is_call()`, `is_put()`, `has_contract_id()`.
Go helper: `RightStr(code int32) string` converts raw right codes to `"C"`/`"P"`/`""`.

Types with contract ID: TradeTick, QuoteTick, OhlcTick, EodTick, OpenInterestTick, TradeQuoteTick, MarketValueTick, GreeksTick, IvTick.

### TradeTick

A single trade execution.

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Milliseconds since midnight ET |
| `sequence` | i32 | Sequence number |
| `ext_condition1` through `ext_condition4` | i32 | Extended trade condition codes |
| `condition` | i32 | Trade condition code |
| `size` | i32 | Trade size (shares/contracts) |
| `exchange` | i32 | Exchange code |
| `price` | f64 | Trade price (decoded) |
| `condition_flags` | i32 | Condition flags bitmap |
| `price_flags` | i32 | Price flags bitmap |
| `volume_type` | i32 | 0 = incremental, 1 = cumulative |
| `records_back` | i32 | Records back count |
| `date` | i32 | Date as YYYYMMDD integer |
| `expiration` | i32 | Contract expiration (wildcard queries) |
| `strike` | f64 | Contract strike (wildcard queries) |
| `right` | i32 (Rust/FFI), string (Go) | Contract right. Rust: C=67/P=80. Go: `"C"`/`"P"`. |

Helper methods: `is_cancelled()`, `trade_condition_no_last()`, `price_condition_set_last()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`, `is_call()`, `is_put()`, `has_contract_id()`

### QuoteTick

An NBBO quote.

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Milliseconds since midnight ET |
| `bid_size` / `ask_size` | i32 | Quote sizes |
| `bid_exchange` / `ask_exchange` | i32 | Exchange codes |
| `bid` / `ask` | f64 | Bid and ask prices (decoded) |
| `bid_condition` / `ask_condition` | i32 | Condition codes |
| `midpoint` | f64 | Pre-computed `(bid + ask) / 2.0` |
| `date` | i32 | Date as YYYYMMDD integer |
| `expiration` / `strike` / `right` | i32/f64/i32 (Go: `right` is string) | Contract ID (wildcard queries) |

Helper methods: `is_call()`, `is_put()`, `has_contract_id()`, plus contract ID helpers

### OhlcTick

An aggregated OHLC bar.

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Bar start time (ms from midnight ET) |
| `open` / `high` / `low` / `close` | f64 | OHLC prices (decoded) |
| `volume` | i32 | Total volume in bar |
| `count` | i32 | Number of trades in bar |
| `date` | i32 | Date as YYYYMMDD integer |
| `expiration` / `strike` / `right` | i32/f64/i32 (Go: `right` is string) | Contract ID (wildcard queries) |

Helper methods: `is_call()`, `is_put()`, `has_contract_id()`, plus contract ID helpers

### EodTick

Full end-of-day snapshot with OHLC + closing quote data.

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` / `ms_of_day2` | i32 | Timestamps |
| `open` / `high` / `low` / `close` | f64 | OHLC prices (decoded) |
| `volume` | i32 | Total daily volume |
| `count` | i32 | Total trade count |
| `bid_size` / `ask_size` | i32 | Closing quote sizes |
| `bid_exchange` / `ask_exchange` | i32 | Closing quote exchanges |
| `bid` / `ask` | f64 | Closing bid/ask (decoded) |
| `bid_condition` / `ask_condition` | i32 | Closing quote conditions |
| `date` | i32 | Date as YYYYMMDD |
| `expiration` / `strike` / `right` | i32/f64/i32 (Go: `right` is string) | Contract ID (wildcard queries) |

Helper methods: `is_call()`, `is_put()`, `has_contract_id()`, plus contract ID helpers

### TradeQuoteTick

Combined trade + quote tick. Contains the full trade data plus the prevailing NBBO quote at the time of the trade.

Helper methods: `is_call()`, `is_put()`, `has_contract_id()`, plus contract ID helpers

### OpenInterestTick

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Milliseconds since midnight ET |
| `open_interest` | i32 | Open interest count |
| `date` | i32 | Date as YYYYMMDD |
| `expiration` / `strike` / `right` | i32/f64/i32 (Go: `right` is string) | Contract ID (wildcard queries) |

### GreeksResult

Result of `all_greeks()`. All fields are `f64`.

| Field | Order | Description |
|-------|-------|-------------|
| `value` | - | Black-Scholes theoretical value |
| `iv` | - | Implied volatility |
| `iv_error` | - | IV solver error (relative) |
| `delta` | 1st | dV/dS |
| `theta` | 1st | dV/dt (daily) |
| `vega` | 1st | dV/dv |
| `rho` | 1st | dV/dr |
| `epsilon` | 1st | dV/dq (dividend sensitivity) |
| `lambda` | 1st | Elasticity (leverage ratio) |
| `gamma` | 2nd | d2V/dS2 |
| `vanna` | 2nd | d2V/dSdv |
| `charm` | 2nd | d2V/dSdt (delta decay) |
| `vomma` | 2nd | d2V/dv2 |
| `veta` | 2nd | d2V/dvdt |
| `speed` | 3rd | d3V/dS3 |
| `zomma` | 3rd | d3V/dS2dv |
| `color` | 3rd | d3V/dS2dt |
| `ultima` | 3rd | d3V/dv3 |
| `d1` | Internal | Black-Scholes d1 |
| `d2` | Internal | Black-Scholes d2 |
| `dual_delta` | Aux | dV/dX |
| `dual_gamma` | Aux | d2V/dX2 |

### Price

All public tick fields are `f64`, decoded at parse time. No price_type conversion is needed in user code.

### SecType

| Variant | Code | String |
|---------|------|--------|
| `Stock` | 0 | `"STOCK"` |
| `Option` | 1 | `"OPTION"` |
| `Index` | 2 | `"INDEX"` |
| `Rate` | 3 | `"RATE"` |

### Right

Option right: `Call`, `Put`

- `from_char('C')` / `from_char('P')` - parse from character
- `as_char()` - convert to `'C'` or `'P'`

**Go SDK:** The `Right` field on all public tick structs is a `string` (`"C"`, `"P"`, or `""`) instead of `i32`. Use `RightStr(code int32)` for manual conversion.

### StreamResponseType

Subscription response codes returned in `ReqResponse` control events.

| Variant | Code | Meaning |
|---------|------|---------|
| `Subscribed` | 0 | Subscription successful |
| `Error` | 1 | General error |
| `MaxStreamsReached` | 2 | Subscription limit reached for your tier |
| `InvalidPerms` | 3 | Insufficient permissions for this data |

### Venue

Data venue for exchange filtering.

| Variant | Wire value |
|---------|------------|
| `Nqb` | `"NQB"` |
| `UtpCta` | `"UTP_CTA"` |

### RateType

Interest rate types for server-side Greeks calculations.

Variants: `Sofr`, `TreasuryM1`, `TreasuryM3`, `TreasuryM6`, `TreasuryY1`, `TreasuryY2`, `TreasuryY3`, `TreasuryY5`, `TreasuryY7`, `TreasuryY10`, `TreasuryY20`, `TreasuryY30`

### Contract

Identifies a security for streaming subscriptions.

::: code-group
```rust [Rust]
Contract::stock("AAPL")
Contract::index("SPX")
Contract::rate("SOFR")
Contract::option("SPY", "20261218", "60", "C")
```
```python [Python]
# Passed as string symbol to subscribe methods
tdx.subscribe_quotes("AAPL")
```
```go [Go]
// Passed as string symbol to subscribe methods
fpss.SubscribeQuotes("AAPL")
```
```cpp [C++]
// Passed as string symbol to subscribe methods
fpss.subscribe_quotes("AAPL");
```
:::

| Field | Type | Description |
|-------|------|-------------|
| `root` | string | Ticker symbol |
| `sec_type` | SecType | Security type |
| `exp_date` | int (optional) | Expiration date as YYYYMMDD (options only) |
| `is_call` | bool (optional) | true = call, false = put (options only) |
| `strike` | string (optional) | Strike price in dollars (options only) |

### Credentials

::: code-group
```rust [Rust]
Credentials::from_file("creds.txt")?;   // line 1 = email, line 2 = password
Credentials::new("user@example.com", "password");
Credentials::parse("user@example.com\npassword")?;
```
```python [Python]
Credentials.from_file("creds.txt")
Credentials("user@example.com", "password")
```
```go [Go]
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
creds := thetadatadx.NewCredentials("email@example.com", "password")
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto creds = tdx::Credentials::from_email("email@example.com", "password");
```
:::

### Error Types

::: code-group
```rust [Rust]
pub enum Error {
    Transport(tonic::transport::Error), // gRPC channel errors
    Status(Box<tonic::Status>),         // gRPC status codes (enriched with ThetaData error info)
    Decompress(String),                 // zstd decompression failure
    Decode(String),                     // protobuf decode failure
    NoData,                             // endpoint returned no usable data
    Auth(String),                       // Nexus auth errors (401/404)
    Fpss(String),                       // FPSS connection errors
    FpssProtocol(String),               // FPSS wire protocol errors
    FpssDisconnected(String),           // FPSS server rejected connection
    Config(String),                     // configuration errors
    Http(reqwest::Error),               // HTTP request errors
    Io(std::io::Error),                 // I/O errors
    Tls(rustls::Error),                 // TLS handshake errors
}
```
```python [Python]
# All errors raise RuntimeError with descriptive message
```
```go [Go]
// All methods return (result, error)
```
```cpp [C++]
// All methods throw std::runtime_error on failure
```
:::

---

## Python-Specific Features

### DataFrame Support

All 61 data methods have `_df` variants that return pandas DataFrames directly:

```python
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
df = tdx.option_history_ohlc_df("SPY", "20241220", "500", "C", "20240315", "60000")
```

Requires `pip install thetadatadx[pandas]`.

### Polars Support

```python
from thetadatadx import to_polars

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df = to_polars(eod)
```

Requires `pip install thetadatadx[polars]`.

### Manual Conversion

```python
from thetadatadx import to_dataframe

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
```
