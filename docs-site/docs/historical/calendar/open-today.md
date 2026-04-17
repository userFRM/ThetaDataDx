---
title: calendar_open_today
description: Check whether the market is open today and get the trading schedule.
---

# calendar_open_today

<TierBadge tier="free" />

Check whether the market is open today and retrieve the current day's trading schedule, including open/close times and any early close indicators.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.calendar_open_today().await?;
for t in &data {
    println!("date={} is_open={} open_time={} close_time={}",
        t.date, t.is_open, t.open_time, t.close_time);
}
```
```python [Python]
data = tdx.calendar_open_today()
for t in data:
    print(f"date={t['date']} is_open={t['is_open']} "
          f"open_time={t['open_time']} close_time={t['close_time']}")
```
```typescript [TypeScript]
const data = tdx.calendarOpenToday();
for (const t of data) {
    console.log(`date=${t.date} is_open=${t.is_open} open_time=${t.open_time} close_time=${t.close_time}`);
}
```
```go [Go]
data, _ := client.CalendarOpenToday()
for _, t := range data {
    fmt.Printf("date=%d is_open=%d open_time=%d close_time=%d\n",
        t.Date, t.IsOpen, t.OpenTime, t.CloseTime)
}
```
```cpp [C++]
auto data = client.calendar_open_today();
for (const auto& t : data) {
    printf("date=%d is_open=%d open_time=%d close_time=%d\n",
        t.date, t.is_open, t.open_time, t.close_time);
}
```
:::

## Parameters

None.

## Response

Returns an array of CalendarDay records with market status fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>is_open</code><span class="param-type">bool</span></div>
<div class="param-desc">Whether the market is open today</div>
</div>
<div class="param">
<div class="param-header"><code>open_time</code><span class="param-type">u32</span></div>
<div class="param-desc">Market open time (milliseconds from midnight ET)</div>
</div>
<div class="param">
<div class="param-header"><code>close_time</code><span class="param-type">u32</span></div>
<div class="param-desc">Market close time (milliseconds from midnight ET)</div>
</div>
<div class="param">
<div class="param-header"><code>early_close</code><span class="param-type">bool</span></div>
<div class="param-desc">Whether today is an early close day</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Today's date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260403, "is_open": 1, "open_time": 34200000, "close_time": 57600000}
]
```

> `is_open=1` means the market is open. `open_time` and `close_time` are milliseconds from midnight ET (34200000 = 9:30 AM, 57600000 = 4:00 PM).

## Notes

- Call this at application startup to determine if live data will be available.
- On holidays, `is_open` will be `false`.
- On early close days (e.g. day before Thanksgiving), `close_time` will be earlier than the standard 4:00 PM ET.
- Reflects NYSE trading hours only.
