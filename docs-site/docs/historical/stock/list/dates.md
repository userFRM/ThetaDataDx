---
title: List Dates
description: Retrieve available dates for a stock by request type (EOD, Trade, Quote, OHLC).
---

# stock_list_dates

List available dates for a stock filtered by request type. Use this to discover what date range is available before requesting historical data.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let dates: Vec<String> = tdx.stock_list_dates("EOD", "AAPL").await?;
println!("First: {} Last: {}", dates.first().unwrap(), dates.last().unwrap());
```
```python [Python]
dates = tdx.stock_list_dates("EOD", "AAPL")
print(f"First: {dates[0]} Last: {dates[-1]}")
```
```go [Go]
dates, err := client.StockListDates("EOD", "AAPL")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("First: %s Last: %s\n", dates[0], dates[len(dates)-1])
```
```cpp [C++]
auto dates = client.stock_list_dates("EOD", "AAPL");
std::cout << "First: " << dates.front()
          << " Last: " << dates.back() << std::endl;
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type: `"EOD"`, `"TRADE"`, `"QUOTE"`, `"OHLC"` |
| `symbol` | string | Yes | Ticker symbol (e.g. `"AAPL"`) |

## Response

List of date strings in `YYYYMMDD` format, sorted chronologically.

## Notes

- The available date range varies by request type. Trade/quote tick data typically covers fewer years than EOD data.
- Use this endpoint to validate date ranges before making history requests.
