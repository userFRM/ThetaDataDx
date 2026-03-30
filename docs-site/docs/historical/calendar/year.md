---
title: calendar_year
description: Get the full trading calendar for a year.
---

# calendar_year

<TierBadge tier="free" />

Retrieve the complete trading calendar for an entire year, including every trading day, holiday, and early close day.

## Code Example

::: code-group
```rust [Rust]
let days: Vec<CalendarDay> = tdx.calendar_year("2024").await?;
```
```python [Python]
result = client.calendar_year("2024")
```
```go [Go]
result, err := client.CalendarYear("2024")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto year_info = client.calendar_year("2024");
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

Returns a `Vec<CalendarDay>` with calendar info for every trading day in the year:

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

- Returns entries for all calendar days in the year, not just trading days. Non-trading days have `is_open: false`.
- Useful for building local trading calendars and scheduling data collection.
- Future years may have incomplete data if the exchange has not yet published the full calendar.
- Reflects NYSE trading hours only.
