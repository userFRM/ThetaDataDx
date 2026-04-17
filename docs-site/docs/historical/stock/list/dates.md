---
title: List Dates
description: Retrieve available dates for a stock by request type (Trade, Quote, OHLC).
---

# stock_list_dates

List available dates for a stock filtered by request type. Use this to discover what date range is available before requesting historical data.

<TierBadge tier="free" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_list_dates("TRADE", "SPY").await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.stock_list_dates("TRADE", "SPY")
for item in data:
    print(item)
```
```typescript [TypeScript]
const data = tdx.stockListDates('TRADE', 'SPY');
console.log(data);
```
```go [Go]
data, _ := client.StockListDates("TRADE", "SPY")
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.stock_list_dates("TRADE", "SPY");
for (const auto& item : data) {
    printf("%s\n", item.c_str());
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>request_type</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Data type: <code>"TRADE"</code>, <code>"QUOTE"</code>, or <code>"OHLC"</code></div>
</div>
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol (e.g. <code>"AAPL"</code>)</div>
</div>
</div>

## Response

List of date strings in `YYYYMMDD` format, sorted chronologically.


### Sample Response

```json
["20040101", "20040102", "20040105", "...", "20260401", "20260402"]
```

> Returns all available dates sorted chronologically. Shown cropped.

## Notes

- The available date range varies by request type.
- Use this endpoint to validate date ranges before making history requests.
