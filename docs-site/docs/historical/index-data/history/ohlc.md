---
title: index_history_ohlc
description: Intraday OHLC bars for an index across a date range.
---

# index_history_ohlc

<TierBadge tier="standard" />

Retrieve intraday OHLC bars for an index across a date range at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let bars: Vec<OhlcTick> = client.index_history_ohlc(
    "SPX", "20240101", "20240301", "60000"  // 1-minute bars
).await?;
for bar in &bars {
    println!("{} {}: O={} H={} L={} C={}",
        bar.date, bar.ms_of_day, bar.open_price(), bar.high_price(),
        bar.low_price(), bar.close_price());
}
```
```python [Python]
bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000")
print(f"{len(bars)} 1-minute bars")

# 5-minute bars
bars_5m = client.index_history_ohlc("SPX", "20240101", "20240301", "300000")
```
```go [Go]
bars, err := client.IndexHistoryOHLC("SPX", "20240101", "20240301", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d bars\n", len(bars))
```
```cpp [C++]
auto bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000");
for (auto& bar : bars) {
    std::cout << bar.date << " " << bar.ms_of_day
              << ": O=" << bar.open << " C=" << bar.close << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol (e.g. `"SPX"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `interval` | string | Yes | Bar interval in milliseconds (e.g. `"60000"` for 1-minute) |
| `start_time` | string | No | Start time of day (ms from midnight) |
| `end_time` | string | No | End time of day (ms from midnight) |

## Response

Returns a list of `OhlcTick` with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `open` | f64 | Opening price |
| `high` | f64 | High price |
| `low` | f64 | Low price |
| `close` | f64 | Closing price |
| `volume` | u64 | Volume |
| `count` | u32 | Number of trades in bar |
| `ms_of_day` | u32 | Milliseconds from midnight ET |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - requires Standard plan or higher.

## Notes

- Common intervals: `"60000"` (1 min), `"300000"` (5 min), `"900000"` (15 min), `"3600000"` (1 hour).
- Use `start_time` and `end_time` to filter to regular trading hours only (e.g. `"34200000"` to `"57600000"` for 9:30 AM to 4:00 PM ET).
- For end-of-day data only, use [index_history_eod](./eod) instead.
