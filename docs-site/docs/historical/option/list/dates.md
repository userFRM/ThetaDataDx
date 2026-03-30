---
title: option_list_dates
description: List available dates for an option contract by request type.
---

# option_list_dates

List available dates for a specific option contract, filtered by data request type. This tells you which dates have data for a given contract.

## Code Example

::: code-group
```rust [Rust]
let dates: Vec<String> = client.option_list_dates(
    "EOD", "SPY", "20241220", "500000", "C"
).await?;
```
```python [Python]
dates = client.option_list_dates("EOD", "SPY", "20241220", "500000", "C")
```
```go [Go]
dates, err := client.OptionListDates("EOD", "SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto dates = client.option_list_dates("EOD", "SPY", "20241220", "500000", "C");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type: `"EOD"`, `"TRADE"`, `"QUOTE"`, `"OHLC"` |
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer, e.g. `"500000"` for $500) |
| `right` | string | Yes | `"C"` for call, `"P"` for put |

## Response

| Field | Type | Description |
|-------|------|-------------|
| (list) | string[] | Date strings in `YYYYMMDD` format |

## Subscription Tier

Free

## Notes

- Different request types may have different date availability. EOD data typically goes back further than tick-level trade or quote data.
- Strike prices are expressed in tenths of a cent: `"500000"` = $500.00.
