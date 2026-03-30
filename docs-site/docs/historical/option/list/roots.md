---
title: option_list_roots
description: List all available option underlying symbols.
---

# option_list_roots

List all available option underlying symbols (roots). Use this to discover which tickers have option chains available in ThetaData.

## Code Example

::: code-group
```rust [Rust]
let symbols: Vec<String> = client.option_list_symbols().await?;
println!("{} option roots available", symbols.len());
```
```python [Python]
symbols = client.option_list_symbols()
print(f"{len(symbols)} option roots available")
```
```go [Go]
symbols, err := client.OptionListSymbols()
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d option roots available\n", len(symbols))
```
```cpp [C++]
auto symbols = client.option_list_symbols();
```
:::

## Parameters

None.

## Response

| Field | Type | Description |
|-------|------|-------------|
| (list) | string[] | Underlying ticker symbols with available option chains |

## Subscription Tier

Free

## Notes

- Returns all underlying symbols, not individual contracts. Use [option_list_expirations](./expirations) and [option_list_strikes](./strikes) to drill into a specific chain.
- The Rust SDK method is `option_list_symbols`; "roots" refers to the underlying concept in ThetaData's API.
