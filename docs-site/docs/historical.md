---
title: Historical Data
description: Access all 61 historical data endpoints for stocks, options, indices, rates, and calendar data across Rust, Python, Go, and C++.
---

# Historical Data

All historical data is accessed through the ThetaDataDx client, which communicates over gRPC with ThetaData's MDDS servers. Every call runs through compiled Rust - gRPC, protobuf parsing, zstd decompression, and FIT decoding all happen at native speed, regardless of which SDK you use.

## Connecting

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let client = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil {
    log.Fatal(err)
}
defer client.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
```
:::

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes, `"3600000"` for 1 hour.

## DataFrame Support (Python)

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

::: code-group
```rust [Rust]
// All available stock symbols
let symbols = client.stock_list_symbols().await?;

// Available dates for a stock by request type
let dates = client.stock_list_dates("TRADE", "AAPL").await?;
```
```python [Python]
# All available stock symbols
symbols = client.stock_list_symbols()

# Available dates by request type
dates = client.stock_list_dates("TRADE", "AAPL")
```
```go [Go]
// All stock symbols
symbols, _ := client.StockListSymbols()

// Available dates by request type
dates, _ := client.StockListDates("TRADE", "AAPL")
```
```cpp [C++]
// All stock symbols
auto symbols = client.stock_list_symbols();

// Available dates by request type
auto dates = client.stock_list_dates("TRADE", "AAPL");
```
:::

### Snapshots

::: code-group
```rust [Rust]
// Latest OHLC snapshot (one or more symbols)
let ticks = client.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;

// Latest trade snapshot
let ticks = client.stock_snapshot_trade(&["AAPL"]).await?;

// Latest NBBO quote snapshot
let ticks = client.stock_snapshot_quote(&["AAPL", "MSFT", "GOOGL"]).await?;
for q in &ticks {
    println!("bid={} ask={}", q.bid, q.ask);
}

// Latest market value snapshot
let ticks = client.stock_snapshot_market_value(&["AAPL"]).await?;
```
```python [Python]
# Latest OHLC snapshot (one or more symbols)
ticks = client.stock_snapshot_ohlc(["AAPL", "MSFT"])

# Latest trade snapshot
ticks = client.stock_snapshot_trade(["AAPL"])

# Latest NBBO quote snapshot
ticks = client.stock_snapshot_quote(["AAPL", "MSFT", "GOOGL"])

# Latest market value
result = client.stock_snapshot_market_value(["AAPL"])
```
```go [Go]
// Latest quote snapshot (multiple symbols)
quotes, _ := client.StockSnapshotQuote([]string{"AAPL", "MSFT", "GOOGL"})
for _, q := range quotes {
    fmt.Printf("bid=%.2f ask=%.2f\n", q.Bid, q.Ask)
}

ohlc, _ := client.StockSnapshotOHLC([]string{"AAPL", "MSFT"})
trades, _ := client.StockSnapshotTrade([]string{"AAPL"})
mv, _ := client.StockSnapshotMarketValue([]string{"AAPL"})
```
```cpp [C++]
// Latest quote snapshot (multiple symbols)
auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOGL"});
for (auto& q : quotes) {
    std::cout << "bid=" << q.bid << " ask=" << q.ask << std::endl;
}

auto ohlc = client.stock_snapshot_ohlc({"AAPL", "MSFT"});
auto trades = client.stock_snapshot_trade({"AAPL"});
auto mv = client.stock_snapshot_market_value({"AAPL"});
```
:::

### History

::: code-group
```rust [Rust]
// End-of-day data for a date range
let eod = client.stock_history_eod("AAPL", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={} V={}",
        t.date, t.open, t.high,
        t.low, t.close, t.volume);
}

// Intraday OHLC bars (single date)
let bars = client.stock_history_ohlc("AAPL", "20240315", "60000").await?;

// Intraday OHLC bars (date range)
let bars = client.stock_history_ohlc_range(
    "AAPL", "20240101", "20240301", "300000"  // 5-min bars
).await?;

// All trades for a date
let trades = client.stock_history_trade("AAPL", "20240315").await?;

// NBBO quotes at a given interval (use "0" for every quote change)
let quotes = client.stock_history_quote("AAPL", "20240315", "60000").await?;

// Combined trade + quote ticks
let ticks = client.stock_history_trade_quote("AAPL", "20240315").await?;
```
```python [Python]
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
```go [Go]
// End-of-day data
eod, _ := client.StockHistoryEOD("AAPL", "20240101", "20240301")
for _, tick := range eod {
    fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
        tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
}

// Intraday OHLC bars
bars, _ := client.StockHistoryOHLC("AAPL", "20240315", "60000")

// OHLC bars across date range
bars, _ = client.StockHistoryOHLCRange("AAPL", "20240101", "20240301", "300000")

// All trades
trades, _ := client.StockHistoryTrade("AAPL", "20240315")

// NBBO quotes
quotes, _ := client.StockHistoryQuote("AAPL", "20240315", "60000")

// Combined trade + quote
result, _ := client.StockHistoryTradeQuote("AAPL", "20240315")
```
```cpp [C++]
// End-of-day data
auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
for (auto& tick : eod) {
    std::cout << tick.date << ": O=" << tick.open
              << " H=" << tick.high << " L=" << tick.low
              << " C=" << tick.close << " V=" << tick.volume << std::endl;
}

// Intraday OHLC bars
auto bars = client.stock_history_ohlc("AAPL", "20240315", "60000");

// OHLC bars across date range
auto range_bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000");

// All trades
auto trades = client.stock_history_trade("AAPL", "20240315");

// NBBO quotes
auto quotes = client.stock_history_quote("AAPL", "20240315", "60000");

// Combined trade + quote
auto tq = client.stock_history_trade_quote("AAPL", "20240315");
```
:::

### At-Time

Retrieve the trade or quote at a specific time of day across a date range. The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

::: code-group
```rust [Rust]
// Trade at a specific time of day across a date range
let trades = client.stock_at_time_trade(
    "AAPL", "20240101", "20240301", "34200000"
).await?;

// Quote at a specific time of day across a date range
let quotes = client.stock_at_time_quote(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
```
```python [Python]
# Trade at a specific time of day across a date range
trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000")

# Quote at a specific time of day
quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000")
```
```go [Go]
// Trade at 9:30 AM across a date range
trades, _ := client.StockAtTimeTrade("AAPL", "20240101", "20240301", "34200000")

// Quote at 9:30 AM
quotes, _ := client.StockAtTimeQuote("AAPL", "20240101", "20240301", "34200000")
```
```cpp [C++]
// Trade at 9:30 AM across a date range
auto trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000");

// Quote at 9:30 AM
auto quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000");
```
:::

### Streaming Large Responses (Rust)

For endpoints returning millions of rows, the Rust SDK provides `_stream` variants to process data chunk by chunk without holding everything in memory:

```rust
client.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;

client.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```

---

## Option Endpoints (34)

### List

::: code-group
```rust [Rust]
// All option underlying symbols
let symbols = client.option_list_symbols().await?;

// Available dates for a specific contract
let dates = client.option_list_dates(
    "TRADE", "SPY", "20240419", "500", "C"
).await?;

// Expiration dates for an underlying
let exps = client.option_list_expirations("SPY").await?;

// Strike prices for a given expiration
let strikes = client.option_list_strikes("SPY", "20240419").await?;

// All contracts for a symbol on a date
let contracts = client.option_list_contracts("TRADE", "SPY", "20240315").await?;
```
```python [Python]
# All option underlying symbols
symbols = client.option_list_symbols()

# Expiration dates for an underlying
exps = client.option_list_expirations("SPY")
print(exps[:10])

# Strike prices for an expiration
strikes = client.option_list_strikes("SPY", "20240419")
print(f"{len(strikes)} strikes")

# Available dates for a contract
dates = client.option_list_dates("TRADE", "SPY", "20240419", "500", "C")

# All contracts for a symbol on a date
contracts = client.option_list_contracts("TRADE", "SPY", "20240315")
```
```go [Go]
symbols, _ := client.OptionListSymbols()
exps, _ := client.OptionListExpirations("SPY")
strikes, _ := client.OptionListStrikes("SPY", "20240419")
dates, _ := client.OptionListDates("TRADE", "SPY", "20240419", "500", "C")
contracts, _ := client.OptionListContracts("TRADE", "SPY", "20240315")
```
```cpp [C++]
auto symbols = client.option_list_symbols();
auto exps = client.option_list_expirations("SPY");
auto strikes = client.option_list_strikes("SPY", "20240419");
auto dates = client.option_list_dates("TRADE", "SPY", "20240419", "500", "C");
auto contracts = client.option_list_contracts("TRADE", "SPY", "20240315");
```
:::

### Snapshots

::: code-group
```rust [Rust]
let ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500", "C").await?;
let trades = client.option_snapshot_trade("SPY", "20240419", "500", "C").await?;
let quotes = client.option_snapshot_quote("SPY", "20240419", "500", "C").await?;
let oi = client.option_snapshot_open_interest("SPY", "20240419", "500", "C").await?;
let mv = client.option_snapshot_market_value("SPY", "20240419", "500", "C").await?;
```
```python [Python]
ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500", "C")
trades = client.option_snapshot_trade("SPY", "20240419", "500", "C")
quotes = client.option_snapshot_quote("SPY", "20240419", "500", "C")
oi = client.option_snapshot_open_interest("SPY", "20240419", "500", "C")
mv = client.option_snapshot_market_value("SPY", "20240419", "500", "C")
```
```go [Go]
ohlc, _ := client.OptionSnapshotOHLC("SPY", "20240419", "500", "C")
trades, _ := client.OptionSnapshotTrade("SPY", "20240419", "500", "C")
quotes, _ := client.OptionSnapshotQuote("SPY", "20240419", "500", "C")
oi, _ := client.OptionSnapshotOpenInterest("SPY", "20240419", "500", "C")
mv, _ := client.OptionSnapshotMarketValue("SPY", "20240419", "500", "C")
```
```cpp [C++]
auto ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500", "C");
auto trades = client.option_snapshot_trade("SPY", "20240419", "500", "C");
auto quotes = client.option_snapshot_quote("SPY", "20240419", "500", "C");
auto oi = client.option_snapshot_open_interest("SPY", "20240419", "500", "C");
auto mv = client.option_snapshot_market_value("SPY", "20240419", "500", "C");
```
:::

### Snapshot Greeks

::: code-group
```rust [Rust]
// All Greeks at once
let all = client.option_snapshot_greeks_all("SPY", "20240419", "500", "C").await?;

// By order
let first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500", "C").await?;
let second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500", "C").await?;
let third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500", "C").await?;

// Just IV
let iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500", "C").await?;
```
```python [Python]
# All Greeks at once
all_g = client.option_snapshot_greeks_all("SPY", "20240419", "500", "C")

# By order
first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500", "C")
second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500", "C")
third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500", "C")

# Just IV
iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500", "C")
```
```go [Go]
all, _ := client.OptionSnapshotGreeksAll("SPY", "20240419", "500", "C")
first, _ := client.OptionSnapshotGreeksFirstOrder("SPY", "20240419", "500", "C")
second, _ := client.OptionSnapshotGreeksSecondOrder("SPY", "20240419", "500", "C")
third, _ := client.OptionSnapshotGreeksThirdOrder("SPY", "20240419", "500", "C")
iv, _ := client.OptionSnapshotGreeksIV("SPY", "20240419", "500", "C")
```
```cpp [C++]
auto all = client.option_snapshot_greeks_all("SPY", "20240419", "500", "C");
auto first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500", "C");
auto second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500", "C");
auto third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500", "C");
auto iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500", "C");
```
:::

### History

::: code-group
```rust [Rust]
// End-of-day option data
let eod = client.option_history_eod(
    "SPY", "20240419", "500", "C", "20240101", "20240301"
).await?;

// Intraday OHLC bars
let bars = client.option_history_ohlc(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;

// All trades for a date
let trades = client.option_history_trade(
    "SPY", "20240419", "500", "C", "20240315"
).await?;

// NBBO quotes at a given interval
let quotes = client.option_history_quote(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;

// Combined trade + quote ticks
let table = client.option_history_trade_quote(
    "SPY", "20240419", "500", "C", "20240315"
).await?;

// Open interest history
let table = client.option_history_open_interest(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
```
```python [Python]
# End-of-day option data
eod = client.option_history_eod("SPY", "20240419", "500", "C",
                                "20240101", "20240301")

# Intraday OHLC bars
bars = client.option_history_ohlc("SPY", "20240419", "500", "C",
                                  "20240315", "60000")

# All trades
trades = client.option_history_trade("SPY", "20240419", "500", "C", "20240315")

# NBBO quotes
quotes = client.option_history_quote("SPY", "20240419", "500", "C",
                                     "20240315", "60000")

# Combined trade + quote ticks
result = client.option_history_trade_quote("SPY", "20240419", "500", "C", "20240315")

# Open interest history
oi = client.option_history_open_interest("SPY", "20240419", "500", "C", "20240315")
```
```go [Go]
eod, _ := client.OptionHistoryEOD("SPY", "20240419", "500", "C", "20240101", "20240301")
bars, _ := client.OptionHistoryOHLC("SPY", "20240419", "500", "C", "20240315", "60000")
trades, _ := client.OptionHistoryTrade("SPY", "20240419", "500", "C", "20240315")
quotes, _ := client.OptionHistoryQuote("SPY", "20240419", "500", "C", "20240315", "60000")
tq, _ := client.OptionHistoryTradeQuote("SPY", "20240419", "500", "C", "20240315")
oi, _ := client.OptionHistoryOpenInterest("SPY", "20240419", "500", "C", "20240315")
```
```cpp [C++]
auto eod = client.option_history_eod("SPY", "20240419", "500", "C", "20240101", "20240301");
auto bars = client.option_history_ohlc("SPY", "20240419", "500", "C", "20240315", "60000");
auto trades = client.option_history_trade("SPY", "20240419", "500", "C", "20240315");
auto quotes = client.option_history_quote("SPY", "20240419", "500", "C", "20240315", "60000");
auto tq = client.option_history_trade_quote("SPY", "20240419", "500", "C", "20240315");
auto oi = client.option_history_open_interest("SPY", "20240419", "500", "C", "20240315");
```
:::

### History Greeks

::: code-group
```rust [Rust]
// EOD Greeks over a date range
let table = client.option_history_greeks_eod(
    "SPY", "20240419", "500", "C", "20240101", "20240301"
).await?;

// Intraday Greeks sampled by interval
let all = client.option_history_greeks_all(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;
let first = client.option_history_greeks_first_order(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;
let second = client.option_history_greeks_second_order(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;
let third = client.option_history_greeks_third_order(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;
let iv = client.option_history_greeks_implied_volatility(
    "SPY", "20240419", "500", "C", "20240315", "60000"
).await?;
```
```python [Python]
# EOD Greeks over a date range
greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500", "C",
                                               "20240101", "20240301")

# Intraday Greeks sampled by interval
all_g = client.option_history_greeks_all("SPY", "20240419", "500", "C",
                                          "20240315", "60000")
first = client.option_history_greeks_first_order("SPY", "20240419", "500", "C",
                                                  "20240315", "60000")
second = client.option_history_greeks_second_order("SPY", "20240419", "500", "C",
                                                    "20240315", "60000")
third = client.option_history_greeks_third_order("SPY", "20240419", "500", "C",
                                                  "20240315", "60000")
iv_hist = client.option_history_greeks_implied_volatility("SPY", "20240419", "500", "C",
                                                           "20240315", "60000")
```
```go [Go]
greeksEOD, _ := client.OptionHistoryGreeksEOD("SPY", "20240419", "500", "C", "20240101", "20240301")
greeksAll, _ := client.OptionHistoryGreeksAll("SPY", "20240419", "500", "C", "20240315", "60000")
greeksFirst, _ := client.OptionHistoryGreeksFirstOrder("SPY", "20240419", "500", "C", "20240315", "60000")
greeksIV, _ := client.OptionHistoryGreeksIV("SPY", "20240419", "500", "C", "20240315", "60000")
```
```cpp [C++]
auto greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500", "C",
                                                    "20240101", "20240301");
auto greeks_all = client.option_history_greeks_all("SPY", "20240419", "500", "C",
                                                    "20240315", "60000");
auto greeks_iv = client.option_history_greeks_implied_volatility("SPY", "20240419", "500", "C",
                                                                  "20240315", "60000");
```
:::

### Trade Greeks

Greeks computed on each individual trade:

::: code-group
```rust [Rust]
let all = client.option_history_trade_greeks_all(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
let first = client.option_history_trade_greeks_first_order(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
let second = client.option_history_trade_greeks_second_order(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
let third = client.option_history_trade_greeks_third_order(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
let iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20240419", "500", "C", "20240315"
).await?;
```
```python [Python]
all_tg = client.option_history_trade_greeks_all("SPY", "20240419", "500", "C", "20240315")
first_tg = client.option_history_trade_greeks_first_order("SPY", "20240419", "500", "C", "20240315")
second_tg = client.option_history_trade_greeks_second_order("SPY", "20240419", "500", "C", "20240315")
third_tg = client.option_history_trade_greeks_third_order("SPY", "20240419", "500", "C", "20240315")
iv_tg = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500", "C", "20240315")
```
```go [Go]
tgAll, _ := client.OptionHistoryTradeGreeksAll("SPY", "20240419", "500", "C", "20240315")
tgFirst, _ := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20240419", "500", "C", "20240315")
tgIV, _ := client.OptionHistoryTradeGreeksIV("SPY", "20240419", "500", "C", "20240315")
```
```cpp [C++]
auto tg_all = client.option_history_trade_greeks_all("SPY", "20240419", "500", "C", "20240315");
auto tg_first = client.option_history_trade_greeks_first_order("SPY", "20240419", "500", "C",
                                                                "20240315");
auto tg_iv = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500", "C",
                                                                    "20240315");
```
:::

### At-Time

::: code-group
```rust [Rust]
let trades = client.option_at_time_trade(
    "SPY", "20240419", "500", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;

let quotes = client.option_at_time_quote(
    "SPY", "20240419", "500", "C",
    "20240101", "20240301", "34200000"
).await?;
```
```python [Python]
trades = client.option_at_time_trade("SPY", "20240419", "500", "C",
                                     "20240101", "20240301", "34200000")
quotes = client.option_at_time_quote("SPY", "20240419", "500", "C",
                                     "20240101", "20240301", "34200000")
```
```go [Go]
trades, _ := client.OptionAtTimeTrade("SPY", "20240419", "500", "C",
    "20240101", "20240301", "34200000")
quotes, _ := client.OptionAtTimeQuote("SPY", "20240419", "500", "C",
    "20240101", "20240301", "34200000")
```
```cpp [C++]
auto trades = client.option_at_time_trade("SPY", "20240419", "500", "C",
                                           "20240101", "20240301", "34200000");
auto quotes = client.option_at_time_quote("SPY", "20240419", "500", "C",
                                           "20240101", "20240301", "34200000");
```
:::

### Streaming Large Option Responses (Rust)

```rust
client.option_history_trade_stream(
    "SPY", "20240419", "500", "C", "20240315",
    |chunk| { Ok(()) }
).await?;

client.option_history_quote_stream(
    "SPY", "20240419", "500", "C", "20240315", "0",
    |chunk| { Ok(()) }
).await?;
```

---

## Index Endpoints (9)

### List

::: code-group
```rust [Rust]
let symbols = client.index_list_symbols().await?;
let dates = client.index_list_dates("SPX").await?;
```
```python [Python]
symbols = client.index_list_symbols()
dates = client.index_list_dates("SPX")
```
```go [Go]
symbols, _ := client.IndexListSymbols()
dates, _ := client.IndexListDates("SPX")
```
```cpp [C++]
auto symbols = client.index_list_symbols();
auto dates = client.index_list_dates("SPX");
```
:::

### Snapshots

::: code-group
```rust [Rust]
let ohlc = client.index_snapshot_ohlc(&["SPX", "NDX"]).await?;
let ticks = client.index_snapshot_price(&["SPX", "NDX"]).await?;
let ticks = client.index_snapshot_market_value(&["SPX"]).await?;
```
```python [Python]
ohlc = client.index_snapshot_ohlc(["SPX", "NDX"])
price = client.index_snapshot_price(["SPX", "NDX"])
mv = client.index_snapshot_market_value(["SPX"])
```
```go [Go]
ohlc, _ := client.IndexSnapshotOHLC([]string{"SPX", "NDX"})
price, _ := client.IndexSnapshotPrice([]string{"SPX"})
mv, _ := client.IndexSnapshotMarketValue([]string{"SPX"})
```
```cpp [C++]
auto ohlc = client.index_snapshot_ohlc({"SPX", "NDX"});
auto price = client.index_snapshot_price({"SPX"});
auto mv = client.index_snapshot_market_value({"SPX"});
```
:::

### History

::: code-group
```rust [Rust]
let eod = client.index_history_eod("SPX", "20240101", "20240301").await?;

let bars = client.index_history_ohlc(
    "SPX", "20240101", "20240301", "60000"
).await?;

let ticks = client.index_history_price("SPX", "20240315", "60000").await?;
```
```python [Python]
eod = client.index_history_eod("SPX", "20240101", "20240301")
df = client.index_history_eod_df("SPX", "20240101", "20240301")
bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000")
price = client.index_history_price("SPX", "20240315", "60000")
```
```go [Go]
eod, _ := client.IndexHistoryEOD("SPX", "20240101", "20240301")
bars, _ := client.IndexHistoryOHLC("SPX", "20240101", "20240301", "60000")
priceHist, _ := client.IndexHistoryPrice("SPX", "20240315", "60000")
```
```cpp [C++]
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
auto bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000");
auto price_hist = client.index_history_price("SPX", "20240315", "60000");
```
:::

### At-Time

::: code-group
```rust [Rust]
let ticks = client.index_at_time_price(
    "SPX", "20240101", "20240301", "34200000"
).await?;
```
```python [Python]
result = client.index_at_time_price("SPX", "20240101", "20240301", "34200000")
```
```go [Go]
atTime, _ := client.IndexAtTimePrice("SPX", "20240101", "20240301", "34200000")
```
```cpp [C++]
auto at_time = client.index_at_time_price("SPX", "20240101", "20240301", "34200000");
```
:::

---

## Rate Endpoints (1)

::: code-group
```rust [Rust]
let rates = client.interest_rate_history_eod(
    "SOFR", "20240101", "20240301"
).await?;
```
```python [Python]
result = client.interest_rate_history_eod("SOFR", "20240101", "20240301")
```
```go [Go]
result, _ := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
```
```cpp [C++]
auto result = client.interest_rate_history_eod("SOFR", "20240101", "20240301");
```
:::

Available rate symbols: `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`, `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`, `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.

---

## Calendar Endpoints (3)

::: code-group
```rust [Rust]
let days = client.calendar_open_today().await?;
let days = client.calendar_on_date("20240315").await?;
let days = client.calendar_year("2024").await?;
```
```python [Python]
result = client.calendar_open_today()
result = client.calendar_on_date("20240315")
result = client.calendar_year("2024")
```
```go [Go]
result, _ := client.CalendarOpenToday()
result, _ = client.CalendarOnDate("20240315")
result, _ = client.CalendarYear("2024")
```
```cpp [C++]
auto today = client.calendar_open_today();
auto date_info = client.calendar_on_date("20240315");
auto year_info = client.calendar_year("2024");
```
:::

---

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |

## Empty Responses

When a query returns no data (e.g., a non-trading date), the SDK returns an empty collection rather than an error. Check for emptiness using the appropriate idiom for your language:

::: code-group
```rust [Rust]
if eod.is_empty() {
    println!("No data for this date range");
}
```
```python [Python]
if not eod:
    print("No data for this date range")
```
```go [Go]
if len(eod) == 0 {
    fmt.Println("No data for this date range")
}
```
```cpp [C++]
if (eod.empty()) {
    std::cout << "No data for this date range" << std::endl;
}
```
:::
