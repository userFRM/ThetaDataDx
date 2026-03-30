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
let table: proto::DataTable = client.calendar_on_date("20240315").await?;
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

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `date` | string | Yes | Date in `YYYYMMDD` format (e.g. `"20240315"`) |

## Response

Returns a `DataTable` with calendar information:

| Field | Type | Description |
|-------|------|-------------|
| `is_open` | bool | Whether the market is open on this date |
| `open_time` | u32 | Market open time (ms from midnight ET) |
| `close_time` | u32 | Market close time (ms from midnight ET) |
| `early_close` | bool | Whether this is an early close day |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - available on all plans.

## Notes

- Use this to check any historical or future date's trading status before requesting data.
- Holidays return `is_open: false`.
- Early close days (e.g. July 3rd, day after Thanksgiving) return a `close_time` earlier than the standard 4:00 PM ET.
- Reflects NYSE trading hours only.
