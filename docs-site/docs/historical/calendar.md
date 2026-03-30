---
title: Calendar & Rates
description: Market calendar and interest rate endpoints - trading schedules, holidays, and risk-free rate history.
---

# Calendar & Rates

## Interest Rate Endpoints (1)

::: code-group
```rust [Rust]
let rates: Vec<InterestRateTick> = tdx.interest_rate_history_eod(
    "SOFR", "20240101", "20240301"
).await?;
```
```python [Python]
result = tdx.interest_rate_history_eod("SOFR", "20240101", "20240301")
```
```go [Go]
result, _ := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
```
```cpp [C++]
auto result = client.interest_rate_history_eod("SOFR", "20240101", "20240301");
```
:::

### Available Rate Symbols

| Symbol | Description |
|--------|-------------|
| `SOFR` | Secured Overnight Financing Rate |
| `TREASURY_M1` | 1-month Treasury |
| `TREASURY_M3` | 3-month Treasury |
| `TREASURY_M6` | 6-month Treasury |
| `TREASURY_Y1` | 1-year Treasury |
| `TREASURY_Y2` | 2-year Treasury |
| `TREASURY_Y3` | 3-year Treasury |
| `TREASURY_Y5` | 5-year Treasury |
| `TREASURY_Y7` | 7-year Treasury |
| `TREASURY_Y10` | 10-year Treasury |
| `TREASURY_Y20` | 20-year Treasury |
| `TREASURY_Y30` | 30-year Treasury |

::: tip
Use interest rate data to set the risk-free rate parameter when computing option Greeks locally.
:::

## Calendar Endpoints (3)

::: code-group
```rust [Rust]
let days: Vec<CalendarDay> = tdx.calendar_open_today().await?;
let days: Vec<CalendarDay> = tdx.calendar_on_date("20240315").await?;
let days: Vec<CalendarDay> = tdx.calendar_year("2024").await?;
```
```python [Python]
result = tdx.calendar_open_today()
result = tdx.calendar_on_date("20240315")
result = tdx.calendar_year("2024")
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

### Calendar Methods

| Method | Description |
|--------|-------------|
| `calendar_open_today` | Check if the market is open today and get the trading schedule |
| `calendar_on_date` | Get the trading schedule for a specific date (regular, early close, or holiday) |
| `calendar_year` | Get the full trading calendar for a year, including all holidays and early closes |

::: warning
Calendar data reflects NYSE trading hours. Other exchanges may have different schedules.
:::
