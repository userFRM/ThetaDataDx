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
let dates: Vec<String> = client.index_list_dates("SPX").await?;
println!("First date: {}, Last date: {}", dates.first().unwrap(), dates.last().unwrap());
```
```python [Python]
dates = client.index_list_dates("SPX")
print(f"Available from {dates[0]} to {dates[-1]}")
```
```go [Go]
dates, err := client.IndexListDates("SPX")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("Available from %s to %s\n", dates[0], dates[len(dates)-1])
```
```cpp [C++]
auto dates = client.index_list_dates("SPX");
std::cout << "Available from " << dates.front() << " to " << dates.back() << std::endl;
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

## Notes

- Use this to determine the date range for which index data is available before making history or EOD calls.
- Dates are returned in ascending order.
