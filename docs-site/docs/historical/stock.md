---
title: Stock Endpoints
description: 14 stock data endpoints - list symbols, snapshots, history, at-time queries, and streaming large responses.
---

# Stock Endpoints (14)

## List

::: code-group
```rust [Rust]
// All available stock symbols
let symbols: Vec<String> = tdx.stock_list_symbols().await?;

// Available dates for a stock by request type
let dates: Vec<String> = tdx.stock_list_dates("EOD", "AAPL").await?;
```
```python [Python]
# All available stock symbols
symbols = tdx.stock_list_symbols()

# Available dates by request type
dates = tdx.stock_list_dates("EOD", "AAPL")
```
```go [Go]
// All stock symbols
symbols, _ := client.StockListSymbols()

// Available dates by request type
dates, _ := client.StockListDates("EOD", "AAPL")
```
```cpp [C++]
// All stock symbols
auto symbols = client.stock_list_symbols();

// Available dates by request type
auto dates = client.stock_list_dates("EOD", "AAPL");
```
:::

## Snapshots

::: code-group
```rust [Rust]
// Latest OHLC snapshot (one or more symbols)
let ticks: Vec<OhlcTick> = tdx.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;

// Latest trade snapshot
let ticks: Vec<TradeTick> = tdx.stock_snapshot_trade(&["AAPL"]).await?;

// Latest NBBO quote snapshot
let ticks: Vec<QuoteTick> = tdx.stock_snapshot_quote(&["AAPL", "MSFT", "GOOGL"]).await?;
for q in &ticks {
    println!("bid={} ask={}", q.bid_price(), q.ask_price());
}

// Latest market value snapshot
let ticks: Vec<MarketValueTick> = tdx.stock_snapshot_market_value(&["AAPL"]).await?;
```
```python [Python]
# Latest OHLC snapshot (one or more symbols)
ticks = tdx.stock_snapshot_ohlc(["AAPL", "MSFT"])

# Latest trade snapshot
ticks = tdx.stock_snapshot_trade(["AAPL"])

# Latest NBBO quote snapshot
ticks = tdx.stock_snapshot_quote(["AAPL", "MSFT", "GOOGL"])

# Latest market value
result = tdx.stock_snapshot_market_value(["AAPL"])
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

::: tip
Snapshot endpoints accept multiple symbols in a single call. Batch your requests to reduce round-trips.
:::

## History

::: code-group
```rust [Rust]
// End-of-day data for a date range
let eod: Vec<EodTick> = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={} V={}",
        t.date, t.open_price(), t.high_price(),
        t.low_price(), t.close_price(), t.volume);
}

// Intraday OHLC bars (single date)
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc("AAPL", "20240315", "60000").await?;

// Intraday OHLC bars (date range)
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc_range(
    "AAPL", "20240101", "20240301", "300000"  // 5-min bars
).await?;

// All trades for a date
let trades: Vec<TradeTick> = tdx.stock_history_trade("AAPL", "20240315").await?;

// NBBO quotes at a given interval (use "0" for every quote change)
let quotes: Vec<QuoteTick> = tdx.stock_history_quote("AAPL", "20240315", "60000").await?;

// Combined trade + quote ticks
let ticks: Vec<TradeQuoteTick> = tdx.stock_history_trade_quote("AAPL", "20240315").await?;
```
```python [Python]
# End-of-day data for a date range
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# As DataFrame
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
print(df.describe())

# Intraday OHLC bars (single date)
bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000")
print(f"{len(bars)} bars")

# Intraday OHLC bars (date range)
bars = tdx.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000")

# All trades for a date
trades = tdx.stock_history_trade("AAPL", "20240315")
print(f"{len(trades)} trades")

# NBBO quotes at a given interval
quotes = tdx.stock_history_quote("AAPL", "20240315", "60000")
df = tdx.stock_history_quote_df("AAPL", "20240315", "0")

# Combined trade + quote ticks
result = tdx.stock_history_trade_quote("AAPL", "20240315")
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

## At-Time

Retrieve the trade or quote at a specific time of day across a date range. The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

::: code-group
```rust [Rust]
// Trade at a specific time of day across a date range
let trades: Vec<TradeTick> = tdx.stock_at_time_trade(
    "AAPL", "20240101", "20240301", "34200000"
).await?;

// Quote at a specific time of day across a date range
let quotes: Vec<QuoteTick> = tdx.stock_at_time_quote(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
```
```python [Python]
# Trade at a specific time of day across a date range
trades = tdx.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000")

# Quote at a specific time of day
quotes = tdx.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000")
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

## Streaming Large Responses (Rust)

For endpoints returning millions of rows, the Rust SDK provides `_stream` variants to process data chunk by chunk without holding everything in memory:

```rust
tdx.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;

tdx.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```

::: tip
Use streaming variants when fetching tick-level data for active symbols. A single day of AAPL trades can exceed 100,000 rows.
:::
