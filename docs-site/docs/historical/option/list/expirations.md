---
title: option_list_expirations
description: List all expiration dates for an underlying symbol.
---

# option_list_expirations

<TierBadge tier="free" />

List all available expiration dates for an underlying symbol. This is typically the first call in an option chain discovery workflow.

## Code Example

::: code-group
```rust [Rust]
let exps: Vec<String> = tdx.option_list_expirations("SPY").await?;
println!("{} expirations available", exps.len());
```
```python [Python]
exps = tdx.option_list_expirations("SPY")
print(exps[:10])
```
```go [Go]
exps, err := client.OptionListExpirations("SPY")
```
```cpp [C++]
auto exps = client.option_list_expirations("SPY");
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>(list)</code><span class="param-type">string[]</span></div>
<div class="param-desc">Expiration date strings in <code>YYYYMMDD</code> format</div>
</div>
</div>

## Notes

- Returns all expirations including weeklies, monthlies, and quarterlies.
- Combine with [option_list_strikes](./strikes) to enumerate the full chain for a given expiration.
