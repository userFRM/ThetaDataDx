---
title: At-Time Trade
description: Retrieve the trade at a specific time of day across a date range.
---

# stock_at_time_trade

Retrieve the trade at a specific time of day across a date range. Returns one trade per date, representing the trade that occurred at or just before the specified time.

The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
// Trade at 9:30 AM across Q1 2024
let trades: Vec<TradeTick> = tdx.stock_at_time_trade(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
for t in &trades {
    println!("{}: price={}", t.date, t.get_price());
}
```
```python [Python]
# Trade at 9:30 AM across Q1 2024
trades = tdx.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000")
for t in trades:
    print(f"{t['date']}: price={t['price']:.2f}")
```
```go [Go]
// Trade at 9:30 AM across Q1 2024
trades, err := client.StockAtTimeTrade("AAPL", "20240101", "20240301", "34200000")
if err != nil {
    log.Fatal(err)
}
for _, t := range trades {
    fmt.Printf("%d: price=%.2f\n", t.Date, t.Price)
}
```
```cpp [C++]
// Trade at 9:30 AM across Q1 2024
auto trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000");
for (auto& t : trades) {
    std::cout << t.date << ": price=" << t.price << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | Milliseconds from midnight ET (e.g. `"34200000"` = 9:30 AM) |
| `venue` | string | No | Data venue filter |

## Response Fields (TradeTick)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Milliseconds since midnight ET |
| `sequence` | i32 | Sequence number |
| `ext_condition1` through `ext_condition4` | i32 | Extended trade condition codes |
| `condition` | i32 | Trade condition code |
| `size` | i32 | Trade size (shares) |
| `exchange` | i32 | Exchange code |
| `price` | i32 | Fixed-point price (use `get_price()`) |
| `condition_flags` | i32 | Condition flags bitmap |
| `price_flags` | i32 | Price flags bitmap |
| `volume_type` | i32 | 0 = incremental, 1 = cumulative |
| `records_back` | i32 | Records back count |
| `price_type` | i32 | Decimal type for price decoding |
| `date` | i32 | Date as YYYYMMDD integer |

Helper methods: `get_price()`, `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`

## Common Time Values

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM (market open) | `"34200000"` |
| 10:00 AM | `"36000000"` |
| 12:00 PM (noon) | `"43200000"` |
| 3:00 PM | `"54000000"` |
| 4:00 PM (market close) | `"57600000"` |

## Notes

- Returns one TradeTick per trading day in the date range.
- Useful for building daily time series at a consistent intraday timestamp (e.g., "price at 10:00 AM every day").
- The returned trade is the one that occurred at or just before the specified time.
