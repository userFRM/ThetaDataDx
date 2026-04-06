---
title: At-Time Quote
description: Retrieve the NBBO quote at a specific time of day across a date range.
---

# stock_at_time_quote

Retrieve the NBBO quote at a specific time of day across a date range. Returns one quote per date, representing the prevailing best bid/ask at the specified time.

The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_at_time_quote("SPY", "20260101", "20260301", "34200000").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} ask={:.2} midpoint={:.2}",
        t.date, t.ms_of_day, t.bid, t.ask, t.midpoint);
}
```
```python [Python]
data = tdx.stock_at_time_quote("SPY", "20260101", "20260301", "34200000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} "
          f"bid={t['bid']:.2f} ask={t['ask']:.2f} midpoint={t['midpoint']:.2f}")
```
```go [Go]
data, _ := client.StockAtTimeQuote("SPY", "20260101", "20260301", "34200000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f midpoint=%.2f\n",
        t.Date, t.MsOfDay, t.Bid, t.Ask, t.Midpoint)
}
```
```cpp [C++]
auto data = client.stock_at_time_quote("SPY", "20260101", "20260301", "34200000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f midpoint=%.2f\n",
        t.date, t.ms_of_day, t.bid, t.ask, t.midpoint);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
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
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Response Fields (QuoteTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Milliseconds since midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code> / <code>ask_size</code><span class="param-type">i32</span></div>
<div class="param-desc">Quote sizes</div>
</div>
<div class="param">
<div class="param-header"><code>bid_exchange</code> / <code>ask_exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Exchange codes</div>
</div>
<div class="param">
<div class="param-header"><code>bid</code> / <code>ask</code><span class="param-type">i32</span></div>
<div class="param-desc">Fixed-point prices. Use <code>bid_price()</code>, <code>ask_price()</code>, <code>midpoint_price()</code> for decoded values.</div>
</div>
<div class="param">
<div class="param-header"><code>bid_condition</code> / <code>ask_condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition codes</div>
</div>
<div class="param">
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `bid_price()`, `ask_price()`, `midpoint_price()`, `midpoint_value()`

## Common Time Values

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM (market open) | `"34200000"` |
| 10:00 AM | `"36000000"` |
| 12:00 PM (noon) | `"43200000"` |
| 3:00 PM | `"54000000"` |
| 4:00 PM (market close) | `"57600000"` |


### Sample Response

```json
[
  {"date": 20260330, "ms_of_day": 43200000, "bid": 637.33, "ask": 637.34, "midpoint": 637.33},
  {"date": 20260331, "ms_of_day": 43200000, "bid": 639.77, "ask": 639.79, "midpoint": 639.78},
  {"date": 20260401, "ms_of_day": 43200000, "bid": 657.81, "ask": 657.82, "midpoint": 657.81}
]
```

> SPY quote at 12:00 PM ET on each date. One row per date in the range.

## Notes

- Returns one QuoteTick per trading day in the date range.
- Useful for building daily spread time series or comparing bid/ask dynamics at a fixed time across trading sessions.
- The returned quote is the NBBO prevailing at the specified time.
