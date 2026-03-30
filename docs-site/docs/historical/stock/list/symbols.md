---
title: List Symbols
description: Retrieve all available stock ticker symbols from ThetaData.
---

# stock_list_symbols

List all available stock ticker symbols.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let symbols: Vec<String> = tdx.stock_list_symbols().await?;
println!("{} symbols available", symbols.len());
```
```python [Python]
symbols = tdx.stock_list_symbols()
print(f"{len(symbols)} symbols available")
```
```go [Go]
symbols, err := client.StockListSymbols()
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d symbols available\n", len(symbols))
```
```cpp [C++]
auto symbols = client.stock_list_symbols();
std::cout << symbols.size() << " symbols available" << std::endl;
```
:::

## Parameters

None.

## Response

List of ticker symbol strings (e.g. `"AAPL"`, `"MSFT"`, `"GOOGL"`).

## Notes

- Returns all symbols for which ThetaData has any historical stock data.
- The list may include delisted symbols with historical data still available.
