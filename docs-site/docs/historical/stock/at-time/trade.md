---
title: At-Time Trade
description: Retrieve the trade at a specific time of day across a date range.
---

# stock_at_time_trade

Retrieve the trade at a specific time of day across a date range. Returns one trade per date, representing the trade that occurred at or just before the specified time.

The `time_of_day` parameter uses ET wall-clock format `HH:MM:SS.SSS` (for example `09:30:00.000`). Legacy millisecond strings such as `34200000` are also accepted.

<TierBadge tier="standard" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_at_time_trade("SPY", "20260101", "20260301", "09:30:00.000").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={}",
        t.date, t.ms_of_day, t.price, t.size);
}
```
```python [Python]
data = tdx.stock_at_time_trade("SPY", "20260101", "20260301", "09:30:00.000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f} size={t['size']}")
```
```typescript [TypeScript]
const data = tdx.stockAtTimeTrade('SPY', '20260101', '20260301', '09:30:00.000');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} price=${t.price} size=${t.size}`);
}
```
```go [Go]
data, _ := client.StockAtTimeTrade("SPY", "20260101", "20260301", "09:30:00.000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f size=%d\n", t.Date, t.MsOfDay, t.Price, t.Size)
}
```
```cpp [C++]
auto data = client.stock_at_time_trade("SPY", "20260101", "20260301", "09:30:00.000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d\n", t.date, t.ms_of_day, t.price, t.size);
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
<div class="param-desc">ET wall-clock time in <code>HH:MM:SS.SSS</code> (e.g. <code>"09:30:00.000"</code>; legacy <code>"34200000"</code> is also accepted)</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Response Fields (TradeTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Milliseconds since midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>sequence</code><span class="param-type">i32</span></div>
<div class="param-desc">Sequence number</div>
</div>
<div class="param">
<div class="param-header"><code>ext_condition1</code> through <code>ext_condition4</code><span class="param-type">i32</span></div>
<div class="param-desc">Extended trade condition codes</div>
</div>
<div class="param">
<div class="param-header"><code>condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade size in shares</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade price (<code>f64</code>, decoded at parse time).</div>
</div>
<div class="param">
<div class="param-header"><code>condition_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>price_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Price flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>volume_type</code><span class="param-type">i32</span></div>
<div class="param-desc"><code>0</code> = incremental volume, <code>1</code> = cumulative volume</div>
</div>
<div class="param">
<div class="param-header"><code>records_back</code><span class="param-type">i32</span></div>
<div class="param-desc">Records back count</div>
</div>
<div class="param">
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`

## Common Time Values

| Time (ET) | `time_of_day` |
|-----------|---------------|
| 9:30 AM (market open) | `"09:30:00.000"` |
| 10:00 AM | `"10:00:00.000"` |
| 12:00 PM (noon) | `"12:00:00.000"` |
| 3:00 PM | `"15:00:00.000"` |
| 4:00 PM (market close) | `"16:00:00.000"` |


### Sample Response

```json
[
  {"date": 20260330, "ms_of_day": 43199998, "price": 637.34, "size": 2140},
  {"date": 20260331, "ms_of_day": 43199983, "price": 639.79, "size": 100},
  {"date": 20260401, "ms_of_day": 43199879, "price": 657.82, "size": 100}
]
```

> SPY trade closest to 12:00 PM ET on each date. One row per date in the range.

## Notes

- Returns one TradeTick per trading day in the date range.
- Useful for building daily time series at a consistent intraday timestamp (e.g., "price at 10:00 AM every day").
- The returned trade is the one that occurred at or just before the specified time.
