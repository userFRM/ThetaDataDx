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
let dates: Vec<String> = tdx.option_list_dates(
    "EOD", "SPY", "20241220", "500000", "C"
).await?;
```
```python [Python]
dates = tdx.option_list_dates("EOD", "SPY", "20241220", "500000", "C")
```
```go [Go]
dates, err := client.OptionListDates("EOD", "SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto dates = client.option_list_dates("EOD", "SPY", "20241220", "500000", "C");
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>request_type</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Data type: <code>"EOD"</code>, <code>"TRADE"</code>, <code>"QUOTE"</code>, or <code>"OHLC"</code></div>
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
<div class="param-desc">Strike price as a scaled integer (e.g. <code>"500000"</code> for $500)</div>
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

## Notes

- Different request types may have different date availability. EOD data typically goes back further than tick-level trade or quote data.
- Strike prices are expressed in tenths of a cent: `"500000"` = $500.00.
