---
title: History EOD
description: End-of-day stock data (OHLC + closing quote) across a date range.
---

# stock_history_eod

End-of-day stock data across a date range. Each row contains the full daily OHLC bar plus closing bid/ask quote data (18 fields total).

<TierBadge tier="free" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_history_eod("SPY", "20260101", "20260301").await?;
for t in &data {
    println!("date={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} bid={:.2} ask={:.2}",
        t.date, t.open, t.high, t.low, t.close, t.volume, t.bid, t.ask);
}
```
```python [Python]
data = tdx.stock_history_eod("SPY", "20260101", "20260301")
for t in data:
    print(f"date={t.date} open={t.open:.2f} high={t.high:.2f} low={t.low:.2f} "
          f"close={t.close:.2f} volume={t.volume} bid={t.bid:.2f} ask={t.ask:.2f}")
```
```typescript [TypeScript]
const data = tdx.stockHistoryEod('SPY', '20260101', '20260301');
for (const t of data) {
    console.log(`date=${t.date} open=${t.open} high=${t.high} low=${t.low} close=${t.close} volume=${t.volume}`);
}
```
```go [Go]
data, _ := client.StockHistoryEOD("SPY", "20260101", "20260301")
for _, t := range data {
    fmt.Printf("date=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d bid=%.2f ask=%.2f\n",
        t.Date, t.Open, t.High, t.Low, t.Close, t.Volume, t.Bid, t.Ask)
}
```
```cpp [C++]
auto data = client.stock_history_eod("SPY", "20260101", "20260301");
for (const auto& t : data) {
    printf("date=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d bid=%.2f ask=%.2f\n",
        t.date, t.open, t.high, t.low, t.close, t.volume, t.bid, t.ask);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol (e.g. <code>"AAPL"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format. Range is inclusive on both ends.</div>
</div>
</div>

## Response Fields (EodTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code> / <code>ms_of_day2</code><span class="param-type">i32</span></div>
<div class="param-desc">Timestamps (milliseconds since midnight Eastern Time)</div>
</div>
<div class="param">
<div class="param-header"><code>open</code> / <code>high</code> / <code>low</code> / <code>close</code><span class="param-type">f64</span></div>
<div class="param-desc">OHLC prices (decoded at parse time).</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">i64</span></div>
<div class="param-desc">Total daily volume</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">i64</span></div>
<div class="param-desc">Total trade count for the day</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code> / <code>ask_size</code><span class="param-type">i32</span></div>
<div class="param-desc">Closing quote sizes</div>
</div>
<div class="param">
<div class="param-header"><code>bid_exchange</code> / <code>ask_exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Closing quote exchange codes</div>
</div>
<div class="param">
<div class="param-header"><code>bid</code> / <code>ask</code><span class="param-type">f64</span></div>
<div class="param-desc">Closing bid/ask prices (decoded at parse time).</div>
</div>
<div class="param">
<div class="param-header"><code>bid_condition</code> / <code>ask_condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Closing quote condition codes</div>
</div>
<div class="param">
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260302, "open": 678.62, "high": 688.62, "low": 678.02, "close": 686.38, "volume": 87115983, "bid": 685.86, "ask": 685.90},
  {"date": 20260303, "open": 674.95, "high": 682.61, "low": 669.66, "close": 680.33, "volume": 104510616, "bid": 679.20, "ask": 679.26},
  {"date": 20260304, "open": 681.58, "high": 687.09, "low": 679.62, "close": 685.13, "volume": 78815016, "bid": 685.45, "ask": 685.60}
]
```

> SPY end-of-day data for March 2026. Full response contains 24 rows.

## Notes

- Python users chain `.to_pandas()` on the return value: `tdx.stock_history_eod(...).to_pandas()`. Requires `pip install thetadatadx[pandas]`.
- EOD data includes the closing NBBO quote alongside OHLCV, making it suitable for strategies that need both price and spread information.
- All dates use `YYYYMMDD` format. The range is inclusive on both ends.
