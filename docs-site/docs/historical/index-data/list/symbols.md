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
let data = tdx.index_list_symbols().await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.index_list_symbols()
for item in data:
    print(item)
```
```go [Go]
data, _ := client.IndexListSymbols()
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.index_list_symbols();
for (const auto& item : data) {
    printf("%s\n", item.c_str());
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


### Sample Response

```json
["SPX", "NDX", "VIX", "RUT", "DJI"]
```

> Returns 13,000+ index symbols. Shown cropped to 5 major indices.

## Notes

- Call this endpoint once at startup to discover available index symbols.
- Results are returned as a flat list of strings.
