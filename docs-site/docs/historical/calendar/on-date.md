---
title: calendar_on_date
description: Get the trading schedule for a specific date.
---

# calendar_on_date

<TierBadge tier="free" />

Retrieve the trading schedule for a specific date, including whether it is a regular trading day, early close, or holiday.

## Code Example

::: code-group
```rust [Rust]
let days: Vec<CalendarDay> = tdx.calendar_on_date("20240315").await?;
```
```python [Python]
result = client.calendar_on_date("20240315")
```
```go [Go]
result, err := client.CalendarOnDate("20240315")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto date_info = client.calendar_on_date("20240315");
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format (e.g. <code>"20240315"</code>)</div>
</div>
</div>

## Response

Returns a `Vec<CalendarDay>` with calendar information:

<div class="param-list">
<div class="param">
<div class="param-header"><code>is_open</code><span class="param-type">bool</span></div>
<div class="param-desc">Whether the market is open on this date</div>
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
<div class="param-desc">Whether this is an early close day</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

## Notes

- Use this to check any historical or future date's trading status before requesting data.
- Holidays return `is_open: false`.
- Early close days (e.g. July 3rd, day after Thanksgiving) return a `close_time` earlier than the standard 4:00 PM ET.
- Reflects NYSE trading hours only.
