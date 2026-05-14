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
```typescript [TypeScript]
const data = tdx.optionListDates('TRADE', 'SPY', '20260417', '550', 'C');
console.log(data);
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
<div class="param-desc">Data type. Upstream enum: <code>"trade"</code>, <code>"quote"</code>. SDK also accepts the legacy upper-case forms <code>"TRADE"</code> / <code>"QUOTE"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> or <code>YYYY-MM-DD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike price in dollars (e.g. <code>"500"</code> or <code>"17.5"</code>), or <code>"*"</code> for all strikes. Default: <code>"*"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Option side: <code>"call"</code>, <code>"put"</code>, or <code>"both"</code>. SDK also accepts <code>"C"</code>/<code>"P"</code>. Default: <code>"both"</code>.</div>
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
