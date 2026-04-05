---
title: option_list_contracts
description: List all option contracts for a symbol on a given date.
---

# option_list_contracts

<TierBadge tier="free" />

List all option contracts available for a given underlying symbol on a specific date. Returns the full matrix of expirations, strikes, and sides.

## Code Example

::: code-group
```rust [Rust]
let contracts: Vec<OptionContract> = tdx.option_list_contracts("TRADE", "SPY", "20240315").await?;
```
```python [Python]
contracts = tdx.option_list_contracts("TRADE", "SPY", "20240315")
```
```go [Go]
contracts, err := client.OptionListContracts("TRADE", "SPY", "20240315")
```
```cpp [C++]
auto contracts = client.option_list_contracts("TRADE", "SPY", "20240315");
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>request_type</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Data type (e.g. <code>"TRADE"</code>, <code>"QUOTE"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration filter</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>root</code><span class="param-type">string</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span></div>
<div class="param-desc">Strike price as scaled integer</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
</div>


### Sample Response

```json
[
  {"symbol": "SPY", "expiration": 20260403, "strike": 320.00, "right": "C"},
  {"symbol": "SPY", "expiration": 20260403, "strike": 640.00, "right": "C"},
  {"symbol": "SPY", "expiration": 20260417, "strike": 550.00, "right": "P"}
]
```

> Lists all option contracts for SPY on the given date. 5,467 contracts returned for 2026-04-02.

## Notes

- Use `max_dte` to limit results to near-term expirations, which significantly reduces the result set for highly liquid underlyings like SPY.
- This is a bulk discovery endpoint. For targeted queries, use [option_list_expirations](./expirations) + [option_list_strikes](./strikes) instead.
