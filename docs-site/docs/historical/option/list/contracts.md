---
title: option_list_contracts
description: List all option contracts for a symbol on a given date.
---

# option_list_contracts

List all option contracts available for a given underlying symbol on a specific date. Returns the full matrix of expirations, strikes, and sides.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.option_list_contracts("EOD", "SPY", "20240315").await?;
```
```python [Python]
contracts = client.option_list_contracts("EOD", "SPY", "20240315")
```
```go [Go]
contracts, err := client.OptionListContracts("EOD", "SPY", "20240315")
```
```cpp [C++]
auto contracts = client.option_list_contracts("EOD", "SPY", "20240315");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `request_type` | string | Yes | Data type (e.g. `"EOD"`, `"TRADE"`) |
| `symbol` | string | Yes | Underlying symbol |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `max_dte` | int | No | Maximum days to expiration filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `root` | string | Underlying symbol |
| `expiration` | string | Expiration date |
| `strike` | string | Strike price (scaled integer) |
| `right` | string | `"C"` or `"P"` |

## Subscription Tier

Free

## Notes

- Use `max_dte` to limit results to near-term expirations, which significantly reduces the result set for highly liquid underlyings like SPY.
- This is a bulk discovery endpoint. For targeted queries, use [option_list_expirations](./expirations) + [option_list_strikes](./strikes) instead.
