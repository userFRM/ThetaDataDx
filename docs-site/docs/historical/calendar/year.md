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
let table: proto::DataTable = client.calendar_year("2024").await?;
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

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `year` | string | Yes | 4-digit year (e.g. `"2024"`) |

## Response

Returns a `DataTable` with calendar info for every trading day in the year:

| Field | Type | Description |
|-------|------|-------------|
| `is_open` | bool | Whether the market is open on this date |
| `open_time` | u32 | Market open time (ms from midnight ET) |
| `close_time` | u32 | Market close time (ms from midnight ET) |
| `early_close` | bool | Whether this is an early close day |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - available on all plans.

## Notes

- Returns entries for all calendar days in the year, not just trading days. Non-trading days have `is_open: false`.
- Useful for building local trading calendars and scheduling data collection.
- Future years may have incomplete data if the exchange has not yet published the full calendar.
- Reflects NYSE trading hours only.
