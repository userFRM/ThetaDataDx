---
title: Snapshot Quote
description: Latest NBBO quote snapshot for one or more stocks.
---

# stock_snapshot_quote

Latest NBBO (National Best Bid and Offer) quote snapshot for one or more stocks.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let quotes: Vec<QuoteTick> = tdx.stock_snapshot_quote(&["AAPL", "MSFT", "GOOGL"]).await?;
for q in &quotes {
    println!("bid={} ask={} spread={:.4}",
        q.bid_price(), q.ask_price(),
        q.ask_price() - q.bid_price());
}
```
```python [Python]
quotes = tdx.stock_snapshot_quote(["AAPL", "MSFT", "GOOGL"])
for q in quotes:
    print(f"bid={q['bid']:.2f} ask={q['ask']:.2f}")
```
```go [Go]
quotes, err := client.StockSnapshotQuote([]string{"AAPL", "MSFT", "GOOGL"})
if err != nil {
    log.Fatal(err)
}
for _, q := range quotes {
    fmt.Printf("bid=%.2f ask=%.2f\n", q.Bid, q.Ask)
}
```
```cpp [C++]
auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOGL"});
for (auto& q : quotes) {
    std::cout << "bid=" << q.bid << " ask=" << q.ask << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight ET) |

## Response Fields (QuoteTick)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Milliseconds since midnight ET |
| `bid_size` / `ask_size` | i32 | Quote sizes |
| `bid_exchange` / `ask_exchange` | i32 | Exchange codes |
| `bid` / `ask` | i32 | Fixed-point prices |
| `bid_condition` / `ask_condition` | i32 | Condition codes |
| `price_type` | i32 | Decimal type for price decoding |
| `date` | i32 | Date as YYYYMMDD integer |

Helper methods: `bid_price()`, `ask_price()`, `midpoint_price()`, `midpoint_value()`

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- The NBBO represents the best bid and ask across all exchanges.
- Use `midpoint_price()` to get the midpoint between bid and ask.
