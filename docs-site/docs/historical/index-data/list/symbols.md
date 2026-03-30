---
title: index_list_symbols
description: List all available index symbols.
---

# index_list_symbols

<TierBadge tier="free" />

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

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbols</code><span class="param-type">string[]</span></div>
<div class="param-desc">List of index ticker symbols (e.g. <code>"SPX"</code>, <code>"NDX"</code>, <code>"DJI"</code>)</div>
</div>
</div>

## Notes

- Call this endpoint once at startup to discover available index symbols.
- Results are returned as a flat list of strings.
