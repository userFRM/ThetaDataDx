---
title: History Quote
description: NBBO quotes for a stock at a configurable sampling interval.
---

# stock_history_quote

NBBO quotes for a stock on a given date, sampled at a configurable interval. Use `"0"` as the interval to get every quote change.

**Tier:** Standard+

## Code Example

::: code-group
```rust [Rust]
// 1-minute sampled quotes
let quotes: Vec<QuoteTick> = tdx.stock_history_quote("AAPL", "20240315", "60000").await?;

// Every quote change (stream variant for large responses)
tdx.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```
```python [Python]
# 1-minute sampled quotes
quotes = tdx.stock_history_quote("AAPL", "20240315", "60000")

# Every quote change as DataFrame
df = tdx.stock_history_quote_df("AAPL", "20240315", "0")
print(f"{len(df)} quote changes")
```
```go [Go]
// 1-minute sampled quotes
quotes, err := client.StockHistoryQuote("AAPL", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d quotes\n", len(quotes))
```
```cpp [C++]
// 1-minute sampled quotes
auto quotes = client.stock_history_quote("AAPL", "20240315", "60000");
std::cout << quotes.size() << " quotes" << std::endl;
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Sampling interval in ms (`"0"` for every change) |
| `start_time` | string | No | Start time (ms from midnight ET) |
| `end_time` | string | No | End time (ms from midnight ET) |
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

## Notes

- Setting `interval` to `"0"` returns every NBBO change, which can produce hundreds of thousands of rows for active symbols. Use the Rust `_stream` variant for large responses.
- Python users can use the `_df` variant for direct DataFrame output: `tdx.stock_history_quote_df(...)`.
- Common intervals: `"60000"` (1 min), `"300000"` (5 min), `"3600000"` (1 hour).
