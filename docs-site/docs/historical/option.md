---
title: Option Endpoints
description: 34 option data endpoints -- list, snapshots, history, Greeks, trade Greeks, and at-time queries.
---

# Option Endpoints (34)

## List

::: code-group
```rust [Rust]
// All option underlying symbols
let symbols: Vec<String> = client.option_list_symbols().await?;

// Available dates for a specific contract
let dates: Vec<String> = client.option_list_dates(
    "EOD", "SPY", "20240419", "500000", "C"
).await?;

// Expiration dates for an underlying
let exps: Vec<String> = client.option_list_expirations("SPY").await?;

// Strike prices for a given expiration
let strikes: Vec<String> = client.option_list_strikes("SPY", "20240419").await?;

// All contracts for a symbol on a date
let table: proto::DataTable = client.option_list_contracts("EOD", "SPY", "20240315").await?;
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
dates = client.option_list_dates("EOD", "SPY", "20240419", "500000", "C")

# All contracts for a symbol on a date
contracts = client.option_list_contracts("EOD", "SPY", "20240315")
```
```go [Go]
symbols, _ := client.OptionListSymbols()
exps, _ := client.OptionListExpirations("SPY")
strikes, _ := client.OptionListStrikes("SPY", "20240419")
dates, _ := client.OptionListDates("EOD", "SPY", "20240419", "500000", "C")
contracts, _ := client.OptionListContracts("EOD", "SPY", "20240315")
```
```cpp [C++]
auto symbols = client.option_list_symbols();
auto exps = client.option_list_expirations("SPY");
auto strikes = client.option_list_strikes("SPY", "20240419");
auto dates = client.option_list_dates("EOD", "SPY", "20240419", "500000", "C");
auto contracts = client.option_list_contracts("EOD", "SPY", "20240315");
```
:::

::: tip
Option contracts are identified by four parameters: underlying symbol, expiration date, strike price (in tenths of a cent, so `500000` = $500.00), and side (`"C"` for call, `"P"` for put).
:::

## Snapshots

::: code-group
```rust [Rust]
let ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C").await?;
let trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C").await?;
let quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C").await?;
let oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C").await?;
let mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C").await?;
```
```python [Python]
ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C")
trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C")
quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C")
oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C")
mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C")
```
```go [Go]
ohlc, _ := client.OptionSnapshotOHLC("SPY", "20240419", "500000", "C")
trades, _ := client.OptionSnapshotTrade("SPY", "20240419", "500000", "C")
quotes, _ := client.OptionSnapshotQuote("SPY", "20240419", "500000", "C")
oi, _ := client.OptionSnapshotOpenInterest("SPY", "20240419", "500000", "C")
mv, _ := client.OptionSnapshotMarketValue("SPY", "20240419", "500000", "C")
```
```cpp [C++]
auto ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C");
auto trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C");
auto quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C");
auto oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C");
auto mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C");
```
:::

## Snapshot Greeks

::: code-group
```rust [Rust]
// All Greeks at once
let all = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C").await?;

// By order
let first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C").await?;
let second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C").await?;
let third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C").await?;

// Just IV
let iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C").await?;
```
```python [Python]
# All Greeks at once
all_g = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C")

# By order
first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C")
second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C")
third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C")

# Just IV
iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C")
```
```go [Go]
all, _ := client.OptionSnapshotGreeksAll("SPY", "20240419", "500000", "C")
first, _ := client.OptionSnapshotGreeksFirstOrder("SPY", "20240419", "500000", "C")
second, _ := client.OptionSnapshotGreeksSecondOrder("SPY", "20240419", "500000", "C")
third, _ := client.OptionSnapshotGreeksThirdOrder("SPY", "20240419", "500000", "C")
iv, _ := client.OptionSnapshotGreeksIV("SPY", "20240419", "500000", "C")
```
```cpp [C++]
auto all = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C");
auto first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C");
auto second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C");
auto third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C");
auto iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C");
```
:::

## History

::: code-group
```rust [Rust]
// End-of-day option data
let eod: Vec<EodTick> = client.option_history_eod(
    "SPY", "20240419", "500000", "C", "20240101", "20240301"
).await?;

// Intraday OHLC bars
let bars: Vec<OhlcTick> = client.option_history_ohlc(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;

// All trades for a date
let trades: Vec<TradeTick> = client.option_history_trade(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;

// NBBO quotes at a given interval
let quotes: Vec<QuoteTick> = client.option_history_quote(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;

// Combined trade + quote ticks
let table = client.option_history_trade_quote(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;

// Open interest history
let table = client.option_history_open_interest(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
```
```python [Python]
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
```go [Go]
eod, _ := client.OptionHistoryEOD("SPY", "20240419", "500000", "C", "20240101", "20240301")
bars, _ := client.OptionHistoryOHLC("SPY", "20240419", "500000", "C", "20240315", "60000")
trades, _ := client.OptionHistoryTrade("SPY", "20240419", "500000", "C", "20240315")
quotes, _ := client.OptionHistoryQuote("SPY", "20240419", "500000", "C", "20240315", "60000")
tq, _ := client.OptionHistoryTradeQuote("SPY", "20240419", "500000", "C", "20240315")
oi, _ := client.OptionHistoryOpenInterest("SPY", "20240419", "500000", "C", "20240315")
```
```cpp [C++]
auto eod = client.option_history_eod("SPY", "20240419", "500000", "C", "20240101", "20240301");
auto bars = client.option_history_ohlc("SPY", "20240419", "500000", "C", "20240315", "60000");
auto trades = client.option_history_trade("SPY", "20240419", "500000", "C", "20240315");
auto quotes = client.option_history_quote("SPY", "20240419", "500000", "C", "20240315", "60000");
auto tq = client.option_history_trade_quote("SPY", "20240419", "500000", "C", "20240315");
auto oi = client.option_history_open_interest("SPY", "20240419", "500000", "C", "20240315");
```
:::

## History Greeks

::: code-group
```rust [Rust]
// EOD Greeks over a date range
let table = client.option_history_greeks_eod(
    "SPY", "20240419", "500000", "C", "20240101", "20240301"
).await?;

// Intraday Greeks sampled by interval
let all = client.option_history_greeks_all(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let first = client.option_history_greeks_first_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let second = client.option_history_greeks_second_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let third = client.option_history_greeks_third_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let iv = client.option_history_greeks_implied_volatility(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
```
```python [Python]
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
```go [Go]
greeksEOD, _ := client.OptionHistoryGreeksEOD("SPY", "20240419", "500000", "C", "20240101", "20240301")
greeksAll, _ := client.OptionHistoryGreeksAll("SPY", "20240419", "500000", "C", "20240315", "60000")
greeksFirst, _ := client.OptionHistoryGreeksFirstOrder("SPY", "20240419", "500000", "C", "20240315", "60000")
greeksIV, _ := client.OptionHistoryGreeksIV("SPY", "20240419", "500000", "C", "20240315", "60000")
```
```cpp [C++]
auto greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500000", "C",
                                                    "20240101", "20240301");
auto greeks_all = client.option_history_greeks_all("SPY", "20240419", "500000", "C",
                                                    "20240315", "60000");
auto greeks_iv = client.option_history_greeks_implied_volatility("SPY", "20240419", "500000", "C",
                                                                  "20240315", "60000");
```
:::

## Trade Greeks

Greeks computed on each individual trade:

::: code-group
```rust [Rust]
let all = client.option_history_trade_greeks_all(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let first = client.option_history_trade_greeks_first_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let second = client.option_history_trade_greeks_second_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let third = client.option_history_trade_greeks_third_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
```
```python [Python]
all_tg = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C", "20240315")
first_tg = client.option_history_trade_greeks_first_order("SPY", "20240419", "500000", "C", "20240315")
second_tg = client.option_history_trade_greeks_second_order("SPY", "20240419", "500000", "C", "20240315")
third_tg = client.option_history_trade_greeks_third_order("SPY", "20240419", "500000", "C", "20240315")
iv_tg = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500000", "C", "20240315")
```
```go [Go]
tgAll, _ := client.OptionHistoryTradeGreeksAll("SPY", "20240419", "500000", "C", "20240315")
tgFirst, _ := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20240419", "500000", "C", "20240315")
tgIV, _ := client.OptionHistoryTradeGreeksIV("SPY", "20240419", "500000", "C", "20240315")
```
```cpp [C++]
auto tg_all = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C", "20240315");
auto tg_first = client.option_history_trade_greeks_first_order("SPY", "20240419", "500000", "C",
                                                                "20240315");
auto tg_iv = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500000", "C",
                                                                    "20240315");
```
:::

## At-Time

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = client.option_at_time_trade(
    "SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;

let quotes: Vec<QuoteTick> = client.option_at_time_quote(
    "SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000"
).await?;
```
```python [Python]
trades = client.option_at_time_trade("SPY", "20240419", "500000", "C",
                                     "20240101", "20240301", "34200000")
quotes = client.option_at_time_quote("SPY", "20240419", "500000", "C",
                                     "20240101", "20240301", "34200000")
```
```go [Go]
trades, _ := client.OptionAtTimeTrade("SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000")
quotes, _ := client.OptionAtTimeQuote("SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000")
```
```cpp [C++]
auto trades = client.option_at_time_trade("SPY", "20240419", "500000", "C",
                                           "20240101", "20240301", "34200000");
auto quotes = client.option_at_time_quote("SPY", "20240419", "500000", "C",
                                           "20240101", "20240301", "34200000");
```
:::

## Streaming Large Option Responses (Rust)

```rust
client.option_history_trade_stream(
    "SPY", "20240419", "500000", "C", "20240315",
    |chunk| { Ok(()) }
).await?;

client.option_history_quote_stream(
    "SPY", "20240419", "500000", "C", "20240315", "0",
    |chunk| { Ok(()) }
).await?;
```
