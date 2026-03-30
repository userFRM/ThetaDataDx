---
title: Snapshot Trade
description: Latest trade snapshot for one or more stocks.
---

# stock_snapshot_trade

Latest trade snapshot for one or more stocks. Returns the most recent trade execution for each symbol.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = tdx.stock_snapshot_trade(&["AAPL"]).await?;
for t in &trades {
    println!("price={} size={}", t.get_price(), t.size);
}
```
```python [Python]
trades = tdx.stock_snapshot_trade(["AAPL"])
for t in trades:
    print(f"price={t['price']:.2f} size={t['size']}")
```
```go [Go]
trades, err := client.StockSnapshotTrade([]string{"AAPL"})
if err != nil {
    log.Fatal(err)
}
for _, t := range trades {
    fmt.Printf("price=%.2f size=%d\n", t.Price, t.Size)
}
```
```cpp [C++]
auto trades = client.stock_snapshot_trade({"AAPL"});
for (auto& t : trades) {
    std::cout << "price=" << t.price
              << " size=" << t.size << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight ET) |

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

- Accepts multiple symbols in a single call.
- Prices are stored as fixed-point integers. Use the `get_price()` helper to get the decoded float value.
