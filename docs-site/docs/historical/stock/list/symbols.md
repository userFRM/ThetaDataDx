---
title: List Symbols
description: Retrieve all available stock ticker symbols from ThetaData.
---

# stock_list_symbols

List all available stock ticker symbols.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_list_symbols().await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.stock_list_symbols()
for item in data:
    print(item)
```
```go [Go]
data, _ := client.StockListSymbols()
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.stock_list_symbols();
for (const auto& item : data) {
    printf("%s\n", item.c_str());
}
```
:::

## Parameters

None.

## Response

List of ticker symbol strings (e.g. `"AAPL"`, `"MSFT"`, `"GOOGL"`).


### Sample Response

```json
["AAPL", "MSFT", "GOOGL", "AMZN", "TSLA"]
```

> Returns 25,000+ symbols. Shown cropped to 5.

## Notes

- Returns all symbols for which ThetaData has any historical stock data.
- The list may include delisted symbols with historical data still available.
