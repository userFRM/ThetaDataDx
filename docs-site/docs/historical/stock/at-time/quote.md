---
title: At-Time Quote
description: Retrieve the NBBO quote at a specific time of day across a date range.
---

# stock_at_time_quote

Retrieve the NBBO quote at a specific time of day across a date range. Returns one quote per date, representing the prevailing best bid/ask at the specified time.

The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
// Quote at 9:30 AM across Q1 2024
let quotes: Vec<QuoteTick> = tdx.stock_at_time_quote(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
for q in &quotes {
    println!("{}: bid={} ask={}", q.date, q.bid_price(), q.ask_price());
}
```
```python [Python]
# Quote at 9:30 AM across Q1 2024
quotes = tdx.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000")
for q in quotes:
    print(f"{q['date']}: bid={q['bid']:.2f} ask={q['ask']:.2f}")
```
```go [Go]
// Quote at 9:30 AM across Q1 2024
quotes, err := client.StockAtTimeQuote("AAPL", "20240101", "20240301", "34200000")
if err != nil {
    log.Fatal(err)
}
for _, q := range quotes {
    fmt.Printf("%d: bid=%.2f ask=%.2f\n", q.Date, q.Bid, q.Ask)
}
```
```cpp [C++]
// Quote at 9:30 AM across Q1 2024
auto quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000");
for (auto& q : quotes) {
    std::cout << q.date << ": bid=" << q.bid
              << " ask=" << q.ask << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | Milliseconds from midnight ET |
| `venue` | string | No | Data venue filter |

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

## Common Time Values

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM (market open) | `"34200000"` |
| 10:00 AM | `"36000000"` |
| 12:00 PM (noon) | `"43200000"` |
| 3:00 PM | `"54000000"` |
| 4:00 PM (market close) | `"57600000"` |

## Notes

- Returns one QuoteTick per trading day in the date range.
- Useful for building daily spread time series or comparing bid/ask dynamics at a fixed time across trading sessions.
- The returned quote is the NBBO prevailing at the specified time.
