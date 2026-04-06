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
let data = tdx.option_list_expirations("SPY").await?;
for item in &data {
    println!("{}", item);
}
```
```python [Python]
data = tdx.option_list_expirations("SPY")
for item in data:
    print(item)
```
```go [Go]
data, _ := client.OptionListExpirations("SPY")
for _, item := range data {
    fmt.Println(item)
}
```
```cpp [C++]
auto data = client.option_list_expirations("SPY");
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
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>(list)</code><span class="param-type">string[]</span></div>
<div class="param-desc">Expiration date strings in <code>YYYYMMDD</code> format</div>
</div>
</div>


### Sample Response

```json
["2012-06-01", "2012-06-08", "2012-06-16", "...", "2028-12-16", "2029-01-19"]
```

> SPY has 2,000+ expirations spanning from 2012 to 2029. Shown cropped.

## Notes

- Returns all expirations including weeklies, monthlies, and quarterlies.
- Combine with [option_list_strikes](./strikes) to enumerate the full chain for a given expiration.
