---
title: index_list_symbols
description: List all available index symbols.
---

# index_list_symbols

List all available index ticker symbols from ThetaData.

## Code Example

::: code-group
```rust [Rust]
let symbols: Vec<String> = client.index_list_symbols().await?;
for sym in &symbols {
    println!("{}", sym);
}
```
```python [Python]
symbols = client.index_list_symbols()
print(symbols)  # e.g. ["SPX", "NDX", "DJI", "VIX", ...]
```
```go [Go]
symbols, err := client.IndexListSymbols()
if err != nil {
    log.Fatal(err)
}
fmt.Println(symbols)
```
```cpp [C++]
auto symbols = client.index_list_symbols();
for (auto& sym : symbols) {
    std::cout << sym << std::endl;
}
```
:::

## Parameters

None.

## Response

| Field | Type | Description |
|-------|------|-------------|
| symbols | string[] | List of index ticker symbols (e.g. `"SPX"`, `"NDX"`, `"DJI"`) |

## Subscription Tier

Free -- available on all plans.

## Notes

- Call this endpoint once at startup to discover available index symbols.
- Results are returned as a flat list of strings.
