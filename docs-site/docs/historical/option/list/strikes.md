---
title: option_list_strikes
description: List strike prices available for a given expiration.
---

# option_list_strikes

List all available strike prices for a given underlying symbol and expiration date.

## Code Example

::: code-group
```rust [Rust]
let strikes: Vec<String> = client.option_list_strikes("SPY", "20241220").await?;
println!("{} strikes available", strikes.len());
```
```python [Python]
strikes = client.option_list_strikes("SPY", "20241220")
print(f"{len(strikes)} strikes")
```
```go [Go]
strikes, err := client.OptionListStrikes("SPY", "20241220")
```
```cpp [C++]
auto strikes = client.option_list_strikes("SPY", "20241220");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |

## Response

| Field | Type | Description |
|-------|------|-------------|
| (list) | string[] | Strike prices as scaled integer strings |

## Subscription Tier

Free

## Notes

- Strike prices are returned as scaled integers in tenths of a cent. Divide by 1000 to get the dollar value: `"500000"` = $500.00.
- Use [option_list_expirations](./expirations) first to get valid expiration dates for an underlying.
