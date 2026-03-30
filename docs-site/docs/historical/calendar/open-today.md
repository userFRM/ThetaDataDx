---
title: calendar_open_today
description: Check whether the market is open today and get the trading schedule.
---

# calendar_open_today

Check whether the market is open today and retrieve the current day's trading schedule, including open/close times and any early close indicators.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.calendar_open_today().await?;
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

Returns a `DataTable` with market status fields:

| Field | Type | Description |
|-------|------|-------------|
| `is_open` | bool | Whether the market is open today |
| `open_time` | u32 | Market open time (ms from midnight ET) |
| `close_time` | u32 | Market close time (ms from midnight ET) |
| `early_close` | bool | Whether today is an early close day |
| `date` | u32 | Today's date as `YYYYMMDD` integer |

## Subscription Tier

Free -- available on all plans.

## Notes

- Call this at application startup to determine if live data will be available.
- On holidays, `is_open` will be `false`.
- On early close days (e.g. day before Thanksgiving), `close_time` will be earlier than the standard 4:00 PM ET.
- Reflects NYSE trading hours only.
