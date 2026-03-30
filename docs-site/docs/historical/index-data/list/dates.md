---
title: index_list_dates
description: List available dates for an index symbol.
---

# index_list_dates

<TierBadge tier="free" />

List all dates for which data is available for a given index symbol.

## Code Example

::: code-group
```rust [Rust]
let dates: Vec<String> = client.index_list_dates("SPX").await?;
println!("First date: {}, Last date: {}", dates.first().unwrap(), dates.last().unwrap());
```
```python [Python]
dates = client.index_list_dates("SPX")
print(f"Available from {dates[0]} to {dates[-1]}")
```
```go [Go]
dates, err := client.IndexListDates("SPX")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("Available from %s to %s\n", dates[0], dates[len(dates)-1])
```
```cpp [C++]
auto dates = client.index_list_dates("SPX");
std::cout << "Available from " << dates.front() << " to " << dates.back() << std::endl;
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol (e.g. `"SPX"`) |

## Response

| Field | Type | Description |
|-------|------|-------------|
| dates | string[] | List of date strings in `YYYYMMDD` format |

 - available on all plans.

## Notes

- Use this to determine the date range for which index data is available before making history or EOD calls.
- Dates are returned in ascending order.
