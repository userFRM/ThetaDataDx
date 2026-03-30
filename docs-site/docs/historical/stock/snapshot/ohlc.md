---
title: Snapshot OHLC
description: Latest OHLC bar snapshot for one or more stocks.
---

# stock_snapshot_ohlc

Latest OHLC (open-high-low-close) snapshot for one or more stocks. Returns the current or most recent trading session's aggregated bar.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let bars: Vec<OhlcTick> = tdx.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;
for bar in &bars {
    println!("O={} H={} L={} C={} V={}",
        bar.open_price(), bar.high_price(),
        bar.low_price(), bar.close_price(), bar.volume);
}
```
```python [Python]
bars = tdx.stock_snapshot_ohlc(["AAPL", "MSFT"])
for bar in bars:
    print(f"O={bar['open']:.2f} H={bar['high']:.2f} "
          f"L={bar['low']:.2f} C={bar['close']:.2f}")
```
```go [Go]
bars, err := client.StockSnapshotOHLC([]string{"AAPL", "MSFT"})
if err != nil {
    log.Fatal(err)
}
for _, bar := range bars {
    fmt.Printf("O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
        bar.Open, bar.High, bar.Low, bar.Close, bar.Volume)
}
```
```cpp [C++]
auto bars = client.stock_snapshot_ohlc({"AAPL", "MSFT"});
for (auto& bar : bars) {
    std::cout << "O=" << bar.open << " H=" << bar.high
              << " L=" << bar.low << " C=" << bar.close
              << " V=" << bar.volume << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight ET) |

## Response Fields (OhlcTick)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | i32 | Bar start time (ms from midnight ET) |
| `open` / `high` / `low` / `close` | i32 | Fixed-point OHLC prices |
| `volume` | i32 | Total volume in bar |
| `count` | i32 | Number of trades in bar |
| `price_type` | i32 | Decimal type for price decoding |
| `date` | i32 | Date as YYYYMMDD integer |

Helper methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- Prices are stored as fixed-point integers. Use the helper methods to get decoded float values.
