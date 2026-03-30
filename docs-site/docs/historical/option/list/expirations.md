---
title: option_list_expirations
description: List all expiration dates for an underlying symbol.
---

# option_list_expirations

List all available expiration dates for an underlying symbol. This is typically the first call in an option chain discovery workflow.

## Code Example

::: code-group
```rust [Rust]
let exps: Vec<String> = client.option_list_expirations("SPY").await?;
println!("{} expirations available", exps.len());
```
```python [Python]
exps = client.option_list_expirations("SPY")
print(exps[:10])
```
```go [Go]
exps, err := client.OptionListExpirations("SPY")
```
```cpp [C++]
auto exps = client.option_list_expirations("SPY");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |

## Response

| Field | Type | Description |
|-------|------|-------------|
| (list) | string[] | Expiration date strings in `YYYYMMDD` format |

## Subscription Tier

Free

## Notes

- Returns all expirations including weeklies, monthlies, and quarterlies.
- Combine with [option_list_strikes](./strikes) to enumerate the full chain for a given expiration.
