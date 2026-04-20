---
title: calendar_year
description: Get the full trading calendar for a year.
---

# calendar_year

<TierBadge tier="value" />

Retrieve the complete trading calendar for an entire year, including every trading day, holiday, and early close day.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.calendar_year("2026").await?;
for t in &data {
    println!("date={} is_open={} open_time={} close_time={}",
        t.date, t.is_open, t.open_time, t.close_time);
}
```
```python [Python]
data = tdx.calendar_year("2026")
for t in data:
    print(f"date={t.date} is_open={t.is_open} "
          f"open_time={t.open_time} close_time={t.close_time}")
```
```typescript [TypeScript]
const data = tdx.calendarYear('2026');
for (const t of data) {
    console.log(`date=${t.date} is_open=${t.is_open} open_time=${t.open_time} close_time=${t.close_time}`);
}
```
```go [Go]
data, _ := client.CalendarYear("2026")
for _, t := range data {
    fmt.Printf("date=%d is_open=%d open_time=%d close_time=%d\n",
        t.Date, t.IsOpen, t.OpenTime, t.CloseTime)
}
```
```cpp [C++]
auto data = client.calendar_year("2026");
for (const auto& t : data) {
    printf("date=%d is_open=%d open_time=%d close_time=%d\n",
        t.date, t.is_open, t.open_time, t.close_time);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>year</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">4-digit year (e.g. <code>"2024"</code>)</div>
</div>
</div>

## Response

Returns an array of CalendarDay records with calendar info for non-standard days in the year (holidays, early closes):

<div class="param-list">
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
<div class="param">
<div class="param-header"><code>is_open</code><span class="param-type">i32</span></div>
<div class="param-desc"><code>1</code> if the market is open, <code>0</code> if closed</div>
</div>
<div class="param">
<div class="param-header"><code>open_time</code><span class="param-type">i32</span></div>
<div class="param-desc">Market open time (milliseconds from midnight ET). <code>0</code> if closed.</div>
</div>
<div class="param">
<div class="param-header"><code>close_time</code><span class="param-type">i32</span></div>
<div class="param-desc">Market close time (milliseconds from midnight ET). <code>0</code> if closed.</div>
</div>
<div class="param">
<div class="param-header"><code>status</code><span class="param-type">i32</span></div>
<div class="param-desc">Day type: <code>0</code> = open, <code>1</code> = early close, <code>2</code> = full close (holiday), <code>3</code> = weekend</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260101, "is_open": 0, "open_time": 0, "close_time": 0},
  {"date": 20260119, "is_open": 0, "open_time": 0, "close_time": 0},
  {"date": 20260216, "is_open": 0, "open_time": 0, "close_time": 0}
]
```

> Returns market holidays for the year. `is_open=0` with `open_time=0` indicates a full closure. Early closures show non-zero `close_time` (e.g., `46800000` = 1:00 PM).

## Notes

- The server returns only non-standard days (holidays and early closes), not every calendar day.
- Useful for building local trading calendars and scheduling data collection.
- Future years may have incomplete data if the exchange has not yet published the full calendar.
- Reflects NYSE trading hours only.
