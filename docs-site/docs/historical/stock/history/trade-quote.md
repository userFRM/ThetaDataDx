---
title: History Trade+Quote
description: Combined trade and prevailing NBBO quote ticks for a stock on a given date.
---

# stock_history_trade_quote

Combined trade + quote ticks for a stock on a given date. Each row contains the full trade data plus the prevailing NBBO quote at the time of the trade.

**Tier:** Pro

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = tdx.stock_history_trade_quote("AAPL", "20240315").await?;
```
```python [Python]
result = tdx.stock_history_trade_quote("AAPL", "20240315")
```
```go [Go]
result, err := client.StockHistoryTradeQuote("AAPL", "20240315")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto tq = client.stock_history_trade_quote("AAPL", "20240315");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight ET) |
| `end_time` | string | No | End time (ms from midnight ET) |
| `exclusive` | bool | No | Exclusive time bounds |
| `venue` | string | No | Data venue filter |

## Response Fields (TradeQuoteTick)

Combined trade + quote tick (25 fields). Contains the full trade data plus the prevailing NBBO quote at the time of the trade.

Helper methods: `trade_price()`, `bid_price()`, `ask_price()`

## Notes

- This endpoint merges trade and quote streams so each trade row includes the best bid/ask at the time of execution. Useful for computing effective spread, price improvement, and trade classification.
- Returns raw DataTable format in Rust. Python returns dicts, Go/C++ return JSON.
- This is a Pro-tier endpoint. Value and Standard subscriptions do not have access.
- The response can be very large for active symbols. Consider filtering with `start_time` / `end_time` or using date ranges that cover only the session you need.
