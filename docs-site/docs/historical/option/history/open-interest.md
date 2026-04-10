---
title: option_history_open_interest
description: Open interest history for an option contract.
---

# option_history_open_interest

<TierBadge tier="free" />

Retrieve open interest history for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_open_interest("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} open_interest={}", t.date, t.open_interest);
}
```
```python [Python]
data = tdx.option_history_open_interest("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t['date']} open_interest={t['open_interest']}")
```
```go [Go]
data, _ := client.OptionHistoryOpenInterest("SPY", "20260417", "550", "C", "20260315")
for _, t := range data {
    fmt.Printf("date=%d open_interest=%d\n", t.Date, t.OpenInterest)
}
```
```cpp [C++]
auto data = client.option_history_open_interest("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d open_interest=%d\n", t.date, t.open_interest);
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
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Strike price in dollars as a string</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>open_interest</code><span class="param-type">int</span></div>
<div class="param-desc">Open interest</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260401, "open_interest": 28},
  {"date": 20260402, "open_interest": 32}
]
```

> Daily open interest for a specific option contract.

## Notes

- Open interest is typically reported once per day based on the previous day's settlement.
