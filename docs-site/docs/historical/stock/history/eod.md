---
title: History EOD
description: End-of-day stock data (OHLC + closing quote) across a date range.
---

# stock_history_eod

End-of-day stock data across a date range. Each row contains the full daily OHLC bar plus closing bid/ask quote data (18 fields total).

<TierBadge tier="value" />

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
<div class="param-header"><code>open</code> / <code>high</code> / <code>low</code> / <code>close</code><span class="param-type">i32</span></div>
<div class="param-desc">Fixed-point OHLC prices. Use <code>open_price()</code>, <code>high_price()</code>, <code>low_price()</code>, <code>close_price()</code> to get decoded <code>f64</code> values.</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">i32</span></div>
<div class="param-desc">Total daily volume</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">i32</span></div>
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
<div class="param-header"><code>bid</code> / <code>ask</code><span class="param-type">i32</span></div>
<div class="param-desc">Closing bid/ask prices (fixed-point). Use <code>bid_price()</code>, <code>ask_price()</code>, <code>midpoint_value()</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>bid_condition</code> / <code>ask_condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Closing quote condition codes</div>
</div>
<div class="param">
<div class="param-header"><code>price_type</code><span class="param-type">i32</span></div>
<div class="param-desc">Decimal type used for fixed-point price decoding</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

## Notes

- Python users can use the `_df` variant to get a pandas DataFrame directly: `tdx.stock_history_eod_df(...)`. Requires `pip install thetadatadx[pandas]`.
- EOD data includes the closing NBBO quote alongside OHLCV, making it suitable for strategies that need both price and spread information.
- All dates use `YYYYMMDD` format. The range is inclusive on both ends.
