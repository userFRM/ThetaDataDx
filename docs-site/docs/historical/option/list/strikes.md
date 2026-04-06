---
title: option_list_strikes
description: List strike prices available for a given expiration.
---

# option_list_strikes

<TierBadge tier="free" />

List all available strike prices for a given underlying symbol and expiration date.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_list_strikes("SPY", "20260417").await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.option_list_strikes("SPY", "20260417")
for item in data:
    print(item)
```
```go [Go]
data, _ := client.OptionListStrikes("SPY", "20260417")
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.option_list_strikes("SPY", "20260417");
for (const auto& item : data) {
    printf("%s\n", item.c_str());
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>(list)</code><span class="param-type">string[]</span></div>
<div class="param-desc">Strike prices as scaled integer strings</div>
</div>
</div>


### Sample Response

```json
["597", "661", "725", "320", "640", "450", "500", "550", "555", "560"]
```

> SPY strikes for the 2026-04-17 expiration. Full response contains 269 strikes.

## Notes

- Strike prices are returned as strings in dollars: `"500"` = $500.00.
- Use [option_list_expirations](./expirations) first to get valid expiration dates for an underlying.
