---
title: History EOD
description: End-of-day stock data (OHLC + closing quote) across a date range.
---

# stock_history_eod

End-of-day stock data across a date range. Each row contains the full daily OHLC bar plus closing bid/ask quote data (18 fields total).

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let eod: Vec<EodTick> = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={} V={}",
        t.date, t.open_price(), t.high_price(),
        t.low_price(), t.close_price(), t.volume);
}
```
```python [Python]
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# As DataFrame
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
print(df.describe())
```
```go [Go]
eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
if err != nil {
    log.Fatal(err)
}
for _, tick := range eod {
    fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
        tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
}
```
```cpp [C++]
auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
for (auto& tick : eod) {
    std::cout << tick.date << ": O=" << tick.open
              << " H=" << tick.high << " L=" << tick.low
              << " C=" << tick.close << " V=" << tick.volume << std::endl;
}
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Ticker symbol (e.g. `"AAPL"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

## Response Fields (EodTick)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` / `ms_of_day2` | i32 | Timestamps |
| `open` / `high` / `low` / `close` | i32 | Fixed-point OHLC prices |
| `volume` | i32 | Total daily volume |
| `count` | i32 | Total trade count |
| `bid_size` / `ask_size` | i32 | Closing quote sizes |
| `bid_exchange` / `ask_exchange` | i32 | Closing quote exchanges |
| `bid` / `ask` | i32 | Closing bid/ask (fixed-point) |
| `bid_condition` / `ask_condition` | i32 | Closing quote conditions |
| `price_type` | i32 | Decimal type |
| `date` | i32 | Date as YYYYMMDD |

Helper methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`, `bid_price()`, `ask_price()`, `midpoint_value()`

## Notes

- Python users can use the `_df` variant to get a pandas DataFrame directly: `tdx.stock_history_eod_df(...)`. Requires `pip install thetadatadx[pandas]`.
- EOD data includes the closing NBBO quote alongside OHLCV, making it suitable for strategies that need both price and spread information.
- All dates use `YYYYMMDD` format. The range is inclusive on both ends.
