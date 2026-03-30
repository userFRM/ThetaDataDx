---
title: index_history_eod
description: End-of-day index data across a date range.
---

# index_history_eod

Retrieve end-of-day data for an index across a date range. Returns one row per trading day with open, high, low, close, and volume.

## Code Example

::: code-group
```rust [Rust]
let eod: Vec<EodTick> = client.index_history_eod("SPX", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={}",
        t.date, t.open_price(), t.high_price(), t.low_price(), t.close_price());
}
```
```python [Python]
eod = client.index_history_eod("SPX", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} C={tick['close']:.2f}")

# DataFrame variant
df = client.index_history_eod_df("SPX", "20240101", "20240301")
print(df.describe())
```
```go [Go]
eod, err := client.IndexHistoryEOD("SPX", "20240101", "20240301")
if err != nil {
    log.Fatal(err)
}
for _, tick := range eod {
    fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
        tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
}
```
```cpp [C++]
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
for (auto& tick : eod) {
    std::cout << tick.date << ": O=" << tick.open << " H=" << tick.high
              << " L=" << tick.low << " C=" << tick.close << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol (e.g. `"SPX"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

## Response

Returns a list of `EodTick` with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `date` | u32 | Date as `YYYYMMDD` integer |
| `open` | f64 | Opening price/level |
| `high` | f64 | High price/level |
| `low` | f64 | Low price/level |
| `close` | f64 | Closing price/level |
| `volume` | u64 | Volume |

## Subscription Tier

Free -- available on all plans.

## Notes

- Returns one row per trading day in the range.
- Non-trading days (weekends, holidays) are excluded from the response.
- Python users can use the `_df` variant to get a pandas DataFrame directly.
