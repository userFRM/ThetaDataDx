---
title: option_history_eod
description: End-of-day option data across a date range.
---

# option_history_eod

<TierBadge tier="free" />

Retrieve end-of-day option data across a date range. Returns one row per trading day with OHLC, volume, and open interest.

## Code Example

::: code-group
```rust [Rust]
let eod: Vec<EodTick> = client.option_history_eod(
    "SPY", "20241220", "500000", "C", "20240101", "20240301"
).await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={}", t.date, t.open_price(), t.high_price(),
        t.low_price(), t.close_price());
}
```
```python [Python]
eod = client.option_history_eod("SPY", "20241220", "500000", "C",
                                "20240101", "20240301")
```
```go [Go]
eod, err := client.OptionHistoryEOD("SPY", "20241220", "500000", "C",
    "20240101", "20240301")
```
```cpp [C++]
auto eod = client.option_history_eod("SPY", "20241220", "500000", "C",
                                      "20240101", "20240301");
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
<div class="param-desc">Strike price as scaled integer</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
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
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Trading date</div>
</div>
<div class="param">
<div class="param-header"><code>open</code><span class="param-type">float</span></div>
<div class="param-desc">Opening price</div>
</div>
<div class="param">
<div class="param-header"><code>high</code><span class="param-type">float</span></div>
<div class="param-desc">High price</div>
</div>
<div class="param">
<div class="param-header"><code>low</code><span class="param-type">float</span></div>
<div class="param-desc">Low price</div>
</div>
<div class="param">
<div class="param-header"><code>close</code><span class="param-type">float</span></div>
<div class="param-desc">Closing price</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">int</span></div>
<div class="param-desc">Daily volume</div>
</div>
<div class="param">
<div class="param-header"><code>open_interest</code><span class="param-type">int</span></div>
<div class="param-desc">Open interest</div>
</div>
</div>
