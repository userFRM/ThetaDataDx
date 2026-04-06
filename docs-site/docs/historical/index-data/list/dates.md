---
title: index_list_dates
description: List available dates for an index symbol.
---

# index_list_dates

<TierBadge tier="free" />

List all dates for which data is available for a given index symbol.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_list_dates("SPX").await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.index_list_dates("SPX")
for item in data:
    print(item)
```
```go [Go]
data, _ := client.IndexListDates("SPX")
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.index_list_dates("SPX");
for (const auto& item : data) {
    printf("%s\n", item.c_str());
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Index symbol (e.g. <code>"SPX"</code>)</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>dates</code><span class="param-type">string[]</span></div>
<div class="param-desc">List of date strings in <code>YYYYMMDD</code> format</div>
</div>
</div>


### Sample Response

```json
["20160801", "20160802", "...", "20260401", "20260402"]
```

> Returns all dates with data for the specified index. SPX has 2,325 dates available.

## Notes

- Use this to determine the date range for which index data is available before making history or EOD calls.
- Dates are returned in ascending order.
