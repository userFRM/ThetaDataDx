---
title: History Trade
description: All trades for a stock on a given date.
---

# stock_history_trade

Retrieve every trade execution for a stock on a given date. Returns tick-level data with price, size, exchange, and condition codes.

**Tier:** Standard+

## Code Example

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = tdx.stock_history_trade("AAPL", "20240315").await?;
println!("{} trades", trades.len());

// Stream variant for large responses
tdx.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;
```
```python [Python]
trades = tdx.stock_history_trade("AAPL", "20240315")
print(f"{len(trades)} trades")
```
```go [Go]
trades, err := client.StockHistoryTrade("AAPL", "20240315")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d trades\n", len(trades))
```
```cpp [C++]
auto trades = client.stock_history_trade("AAPL", "20240315");
std::cout << trades.size() << " trades" << std::endl;
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight ET) |
| `end_time` | string | No | End time (ms from midnight ET) |
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

## Notes

- A single day of AAPL trades can exceed 100,000 rows. Use the Rust `_stream` variant for large responses to avoid holding everything in memory.
- Use `start_time` and `end_time` to limit to regular trading hours (9:30 AM = `34200000`, 4:00 PM = `57600000`).
- The `condition` and `condition_flags` fields encode SIP trade condition codes (e.g., regular sale, odd lot, intermarket sweep).
