---
title: Index Endpoints
description: 9 index data endpoints - list symbols, snapshots, history, and at-time queries for market indices.
---

# Index Endpoints (9)

## List

::: code-group
```rust [Rust]
let symbols: Vec<String> = client.index_list_symbols().await?;
let dates: Vec<String> = client.index_list_dates("SPX").await?;
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

## Snapshots

::: code-group
```rust [Rust]
let ohlc: Vec<OhlcTick> = client.index_snapshot_ohlc(&["SPX", "NDX"]).await?;
let ticks: Vec<PriceTick> = tdx.index_snapshot_price(&["SPX", "NDX"]).await?;
let ticks: Vec<MarketValueTick> = tdx.index_snapshot_market_value(&["SPX"]).await?;
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

## History

::: code-group
```rust [Rust]
let eod: Vec<EodTick> = client.index_history_eod("SPX", "20240101", "20240301").await?;

let bars: Vec<OhlcTick> = client.index_history_ohlc(
    "SPX", "20240101", "20240301", "60000"
).await?;

let ticks: Vec<PriceTick> = tdx.index_history_price("SPX", "20240315", "60000").await?;
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

## At-Time

::: code-group
```rust [Rust]
let ticks: Vec<PriceTick> = tdx.index_at_time_price(
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
