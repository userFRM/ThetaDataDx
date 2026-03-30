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
let days: Vec<CalendarDay> = tdx.calendar_open_today().await?;
```
```python [Python]
result = client.calendar_open_today()
```
```go [Go]
result, err := client.CalendarOpenToday()
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto today = client.calendar_open_today();
```
:::

## Parameters

None.

## Response

Returns a `Vec<CalendarDay>` with market status fields:

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

## Notes

- Call this at application startup to determine if live data will be available.
- On holidays, `is_open` will be `false`.
- On early close days (e.g. day before Thanksgiving), `close_time` will be earlier than the standard 4:00 PM ET.
- Reflects NYSE trading hours only.
