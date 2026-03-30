---
title: At-Time Trade
description: Retrieve the trade at a specific time of day across a date range.
---

# stock_at_time_trade

Retrieve the trade at a specific time of day across a date range. Returns one trade per date, representing the trade that occurred at or just before the specified time.

The `time_of_day` parameter is milliseconds from midnight ET (e.g., `34200000` = 9:30 AM).

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
// Trade at 9:30 AM across Q1 2024
let trades: Vec<TradeTick> = tdx.stock_at_time_trade(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
for t in &trades {
    println!("{}: price={}", t.date, t.get_price());
}
```
```python [Python]
# Trade at 9:30 AM across Q1 2024
trades = tdx.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000")
for t in trades:
    print(f"{t['date']}: price={t['price']:.2f}")
```
```go [Go]
// Trade at 9:30 AM across Q1 2024
trades, err := client.StockAtTimeTrade("AAPL", "20240101", "20240301", "34200000")
if err != nil {
    log.Fatal(err)
}
for _, t := range trades {
    fmt.Printf("%d: price=%.2f\n", t.Date, t.Price)
}
```
```cpp [C++]
// Trade at 9:30 AM across Q1 2024
auto trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000");
for (auto& t : trades) {
    std::cout << t.date << ": price=" << t.price << std::endl;
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>time_of_day</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Milliseconds from midnight ET (e.g. <code>"34200000"</code> = 9:30 AM)</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Response Fields (TradeTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Milliseconds since midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>sequence</code><span class="param-type">i32</span></div>
<div class="param-desc">Sequence number</div>
</div>
<div class="param">
<div class="param-header"><code>ext_condition1</code> through <code>ext_condition4</code><span class="param-type">i32</span></div>
<div class="param-desc">Extended trade condition codes</div>
</div>
<div class="param">
<div class="param-header"><code>condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade size in shares</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">i32</span></div>
<div class="param-desc">Fixed-point price. Use <code>get_price()</code> for decoded <code>f64</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>condition_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>price_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Price flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>volume_type</code><span class="param-type">i32</span></div>
<div class="param-desc"><code>0</code> = incremental volume, <code>1</code> = cumulative volume</div>
</div>
<div class="param">
<div class="param-header"><code>records_back</code><span class="param-type">i32</span></div>
<div class="param-desc">Records back count</div>
</div>
<div class="param">
<div class="param-header"><code>price_type</code><span class="param-type">i32</span></div>
<div class="param-desc">Decimal type for price decoding</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `get_price()`, `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`

## Common Time Values

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM (market open) | `"34200000"` |
| 10:00 AM | `"36000000"` |
| 12:00 PM (noon) | `"43200000"` |
| 3:00 PM | `"54000000"` |
| 4:00 PM (market close) | `"57600000"` |

## Notes

- Returns one TradeTick per trading day in the date range.
- Useful for building daily time series at a consistent intraday timestamp (e.g., "price at 10:00 AM every day").
- The returned trade is the one that occurred at or just before the specified time.
