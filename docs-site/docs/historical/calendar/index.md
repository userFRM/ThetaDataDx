---
title: Calendar Endpoints
description: Market calendar endpoints - trading schedules, holidays, and early closes.
---

# Calendar Endpoints (3)

Market calendar data for determining trading schedules, holidays, and early close days. All calendar data reflects NYSE trading hours.

## Endpoints

| Endpoint | Description |
|----------|-------------|
| [calendar_open_today](./open-today) | Check if the market is open today |
| [calendar_on_date](./on-date) | Get the trading schedule for a specific date |
| [calendar_year](./year) | Get the full trading calendar for a year |

## Quick Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.calendar_open_today().await?;
let table: proto::DataTable = client.calendar_on_date("20240315").await?;
let table: proto::DataTable = client.calendar_year("2024").await?;
```
```python [Python]
result = client.calendar_open_today()
result = client.calendar_on_date("20240315")
result = client.calendar_year("2024")
```
```go [Go]
result, _ := client.CalendarOpenToday()
result, _ = client.CalendarOnDate("20240315")
result, _ = client.CalendarYear("2024")
```
```cpp [C++]
auto today = client.calendar_open_today();
auto date_info = client.calendar_on_date("20240315");
auto year_info = client.calendar_year("2024");
```
:::

::: warning
Calendar data reflects NYSE trading hours. Other exchanges may have different schedules.
:::
