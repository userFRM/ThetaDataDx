---
title: option_list_dates
description: List available dates for an option contract by request type.
---

# option_list_dates

<TierBadge tier="free" />

List available dates for a specific option contract, filtered by data request type. This tells you which dates have data for a given contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_list_dates("TRADE", "SPY", "20260417", "550", "C").await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.option_list_dates("TRADE", "SPY", "20260417", "550", "C")
for item in data:
    print(item)
```
```go [Go]
data, _ := client.OptionListDates("TRADE", "SPY", "20260417", "550", "C")
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.option_list_dates("TRADE", "SPY", "20260417", "550", "C");
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
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Strike price as a scaled integer (e.g. <code>"500"</code> for $500)</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>(list)</code><span class="param-type">string[]</span></div>
<div class="param-desc">Date strings in <code>YYYYMMDD</code> format</div>
</div>
</div>


### Sample Response

```json
["20260301", "20260302", "20260303", "...", "20260401", "20260402"]
```

> All dates with data for the specified contract and request type.

## Notes

- Different request types may have different date availability.
- Strike prices are expressed in dollars as a string: `"500"` = $500.00.
