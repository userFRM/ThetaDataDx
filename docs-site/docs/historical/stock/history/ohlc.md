---
title: History OHLC
description: Intraday OHLC bars for a single date or across a date range.
---

# stock_history_ohlc / stock_history_ohlc_range

Intraday OHLC bars at a configurable interval. Two variants are available:

- **stock_history_ohlc** -- bars for a single date
- **stock_history_ohlc_range** -- bars across a date range

**Tier:** Value+

## Code Example (Single Date)

::: code-group
```rust [Rust]
// 1-minute bars for a single date
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc("AAPL", "20240315", "60000").await?;
println!("{} bars", bars.len());
```
```python [Python]
# 1-minute bars for a single date
bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000")
print(f"{len(bars)} bars")
```
```go [Go]
// 1-minute bars for a single date
bars, err := client.StockHistoryOHLC("AAPL", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d bars\n", len(bars))
```
```cpp [C++]
// 1-minute bars for a single date
auto bars = client.stock_history_ohlc("AAPL", "20240315", "60000");
std::cout << bars.size() << " bars" << std::endl;
```
:::

## Code Example (Date Range)

::: code-group
```rust [Rust]
// 5-minute bars across a date range
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc_range(
    "AAPL", "20240101", "20240301", "300000"
).await?;
```
```python [Python]
# 5-minute bars across a date range
bars = tdx.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000")
```
```go [Go]
// 5-minute bars across a date range
bars, err := client.StockHistoryOHLCRange("AAPL", "20240101", "20240301", "300000")
```
```cpp [C++]
// 5-minute bars across a date range
auto bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000");
```
:::

## Parameters (Single Date)

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Bar interval in milliseconds (e.g. `"60000"` for 1-minute) |
| `start_time` | string | No | Start time (ms from midnight ET) |
| `end_time` | string | No | End time (ms from midnight ET) |
| `venue` | string | No | Data venue filter |

## Parameters (Date Range)

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `interval` | string | Yes | Bar interval in milliseconds |

## Response Fields (OhlcTick)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Bar start time (ms from midnight ET) |
| `open` / `high` / `low` / `close` | i32 | Fixed-point OHLC prices |
| `volume` | i32 | Total volume in bar |
| `count` | i32 | Number of trades in bar |
| `price_type` | i32 | Decimal type for price decoding |
| `date` | i32 | Date as YYYYMMDD integer |

Helper methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`

## Common Intervals

| Interval | Milliseconds |
|----------|-------------|
| 1 minute | `"60000"` |
| 5 minutes | `"300000"` |
| 15 minutes | `"900000"` |
| 1 hour | `"3600000"` |

## Notes

- Use the single-date variant for intraday analysis of a specific session.
- Use the range variant for building multi-day bar charts or backtesting.
- Optional `start_time` / `end_time` parameters (single-date variant only) let you filter to regular trading hours or a custom window.
