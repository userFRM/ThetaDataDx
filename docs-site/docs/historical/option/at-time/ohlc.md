---
title: option_at_time_quote
description: Quote at a specific time of day across a date range for an option contract.
---

# option_at_time_quote

<TierBadge tier="free" />

Retrieve the NBBO quote at a specific time of day across a date range for an option contract. Returns one quote per date, the prevailing quote at the specified time.

## Code Example

::: code-group
```rust [Rust]
let quotes: Vec<QuoteTick> = tdx.option_at_time_quote(
    "SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;
```
```python [Python]
quotes = tdx.option_at_time_quote("SPY", "20241220", "500000", "C",
                                     "20240101", "20240301", "34200000")
```
```go [Go]
quotes, err := client.OptionAtTimeQuote("SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000")
```
```cpp [C++]
auto quotes = client.option_at_time_quote("SPY", "20241220", "500000", "C",
                                           "20240101", "20240301", "34200000");
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
<div class="param-header"><code>time_of_day</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Milliseconds from midnight ET (e.g. <code>"34200000"</code> = 9:30 AM)</div>
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
<div class="param-header"><code>bid_price</code><span class="param-type">float</span></div>
<div class="param-desc">Best bid price</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code><span class="param-type">int</span></div>
<div class="param-desc">Bid size</div>
</div>
<div class="param">
<div class="param-header"><code>ask_price</code><span class="param-type">float</span></div>
<div class="param-desc">Best ask price</div>
</div>
<div class="param">
<div class="param-header"><code>ask_size</code><span class="param-type">int</span></div>
<div class="param-desc">Ask size</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">int</span></div>
<div class="param-desc">Milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>bid_exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Bid exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>ask_exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Ask exchange code</div>
</div>
</div>

## Notes

- Common time values: `"34200000"` (9:30 AM), `"46800000"` (1:00 PM), `"57600000"` (4:00 PM).
- Useful for building daily spread or mid-price time series at a consistent intraday timestamp.
