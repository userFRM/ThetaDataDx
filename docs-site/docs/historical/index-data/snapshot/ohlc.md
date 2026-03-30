---
title: index_snapshot_ohlc
description: Latest OHLC snapshot for one or more indices.
---

# index_snapshot_ohlc

Get the latest OHLC (open, high, low, close) snapshot for one or more index symbols. Returns the most recent bar data.

## Code Example

::: code-group
```rust [Rust]
let bars: Vec<OhlcTick> = client.index_snapshot_ohlc(&["SPX", "VIX"]).await?;
for bar in &bars {
    println!("O={} H={} L={} C={}", bar.open_price(), bar.high_price(), bar.low_price(), bar.close_price());
}
```
```python [Python]
bars = client.index_snapshot_ohlc(["SPX", "VIX"])
for bar in bars:
    print(f"O={bar['open']:.2f} H={bar['high']:.2f} L={bar['low']:.2f} C={bar['close']:.2f}")
```
```go [Go]
bars, err := client.IndexSnapshotOHLC([]string{"SPX", "VIX"})
if err != nil {
    log.Fatal(err)
}
for _, bar := range bars {
    fmt.Printf("O=%.2f H=%.2f L=%.2f C=%.2f\n", bar.Open, bar.High, bar.Low, bar.Close)
}
```
```cpp [C++]
auto bars = client.index_snapshot_ohlc({"SPX", "VIX"});
for (auto& bar : bars) {
    std::cout << "O=" << bar.open << " H=" << bar.high
              << " L=" << bar.low << " C=" << bar.close << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

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

## Subscription Tier

Value -- requires Value plan or higher.

## Notes

- Pass multiple symbols in a single call to batch requests efficiently.
- During market hours, the snapshot reflects the current partial bar.
