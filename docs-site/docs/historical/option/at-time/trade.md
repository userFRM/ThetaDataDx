---
title: option_at_time_trade
description: Trade at a specific time of day across a date range for an option contract.
---

# option_at_time_trade

<TierBadge tier="free" />

Retrieve the trade at a specific time of day across a date range for an option contract. Returns one trade per date, the most recent trade at or before the specified time.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_at_time_trade("SPY", "20260417", "550", "C", "20260101", "20260301", "09:30:00.000").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={}",
        t.date, t.ms_of_day, t.price, t.size);
}
```
```python [Python]
data = tdx.option_at_time_trade("SPY", "20260417", "550", "C", "20260101", "20260301", "09:30:00.000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f} size={t['size']}")
```
```go [Go]
data, _ := client.OptionAtTimeTrade("SPY", "20260417", "550", "C", "20260101", "20260301", "09:30:00.000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f size=%d\n", t.Date, t.MsOfDay, t.Price, t.Size)
}
```
```cpp [C++]
auto data = client.option_at_time_trade("SPY", "20260417", "550", "C", "20260101", "20260301", "09:30:00.000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d\n", t.date, t.ms_of_day, t.price, t.size);
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
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>time_of_day</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">ET wall-clock time in <code>HH:MM:SS.SSS</code> (e.g. <code>"09:30:00.000"</code>; legacy <code>"34200000"</code> is also accepted)</div>
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
<div class="param-header"><code>price</code><span class="param-type">float</span></div>
<div class="param-desc">Trade price</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">int</span></div>
<div class="param-desc">Trade size</div>
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
<div class="param-header"><code>condition</code><span class="param-type">int</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Exchange code</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260331, "ms_of_day": 38482216, "price": 93.00, "size": 1},
  {"date": 20260402, "ms_of_day": 34203497, "price": 98.59, "size": 1}
]
```

> Trade closest to 12:00 PM ET for SPY 2026-04-17 550 call. One row per date.

## Notes

- Common time values: `"09:30:00.000"` (9:30 AM), `"13:00:00.000"` (1:00 PM), `"16:00:00.000"` (4:00 PM).
- Useful for building daily time series at a consistent intraday timestamp (e.g., opening trade every day).
