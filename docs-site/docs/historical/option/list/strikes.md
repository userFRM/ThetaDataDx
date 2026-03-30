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
let strikes: Vec<String> = client.option_list_strikes("SPY", "20241220").await?;
println!("{} strikes available", strikes.len());
```
```python [Python]
strikes = client.option_list_strikes("SPY", "20241220")
print(f"{len(strikes)} strikes")
```
```go [Go]
strikes, err := client.OptionListStrikes("SPY", "20241220")
```
```cpp [C++]
auto strikes = client.option_list_strikes("SPY", "20241220");
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

## Notes

- Strike prices are returned as scaled integers in tenths of a cent. Divide by 1000 to get the dollar value: `"500000"` = $500.00.
- Use [option_list_expirations](./expirations) first to get valid expiration dates for an underlying.
