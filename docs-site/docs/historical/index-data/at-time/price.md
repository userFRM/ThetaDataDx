---
title: index_at_time_price
description: Index price at a specific time of day across a date range.
---

# index_at_time_price

<TierBadge tier="standard" />

Retrieve the index price at a specific time of day for every trading day in a date range. Returns one data point per date, useful for consistent daily sampling.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_at_time_price("SPX", "20260101", "20260301", "34200000").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2}", t.date, t.ms_of_day, t.price_f64());
}
```
```python [Python]
data = tdx.index_at_time_price("SPX", "20260101", "20260301", "34200000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f}")
```
```go [Go]
data, _ := client.IndexAtTimePrice("SPX", "20260101", "20260301", "34200000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f\n", t.Date, t.MsOfDay, t.Price)
}
```
```cpp [C++]
auto data = client.index_at_time_price("SPX", "20260101", "20260301", "34200000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f\n", t.date, t.ms_of_day, t.price);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Index symbol (e.g. <code>"SPX"</code>)</div>
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
<div class="param-desc">Milliseconds from midnight ET (e.g. <code>"34200000"</code> for 9:30 AM)</div>
</div>
</div>

## Response

Returns a `Vec<PriceTick>` with one entry per trading day:

<div class="param-list">
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">f64</span></div>
<div class="param-desc">Index price/level at the specified time</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">u32</span></div>
<div class="param-desc">Actual milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |


### Sample Response

```json
[
  {"date": 20260330, "ms_of_day": 43200000, "price": 6742.18},
  {"date": 20260331, "ms_of_day": 43200000, "price": 6768.45},
  {"date": 20260401, "ms_of_day": 43200000, "price": 6891.33}
]
```

> SPX price at 12:00 PM ET on each date. One row per date in the range. Requires Value subscription.

## Notes

- Returns the price at or just before the specified time of day.
- Useful for building daily time series at a consistent sample point (e.g. market open, noon, close).
- Non-trading days are excluded from the response.
