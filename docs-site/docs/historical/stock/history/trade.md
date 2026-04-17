---
title: History Trade
description: All trades for a stock on a given date.
---

# stock_history_trade

Retrieve every trade execution for a stock on a given date. Returns tick-level data with price, size, exchange, and condition codes.

<TierBadge tier="standard" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_history_trade("SPY", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={} exchange={} condition={} sequence={}",
        t.date, t.ms_of_day, t.price, t.size, t.exchange, t.condition, t.sequence);
}
```
```python [Python]
data = tdx.stock_history_trade("SPY", "20260315")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f} "
          f"size={t['size']} exchange={t['exchange']} condition={t['condition']} sequence={t['sequence']}")
```
```typescript [TypeScript]
const data = tdx.stockHistoryTrade('SPY', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} price=${t.price} size=${t.size} exchange=${t.exchange} condition=${t.condition}`);
}
```
```go [Go]
data, _ := client.StockHistoryTrade("SPY", "20260315")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f size=%d exchange=%d condition=%d sequence=%d\n",
        t.Date, t.MsOfDay, t.Price, t.Size, t.Exchange, t.Condition, t.Sequence)
}
```
```cpp [C++]
auto data = client.stock_history_trade("SPY", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d exchange=%d condition=%d sequence=%d\n",
        t.date, t.ms_of_day, t.price, t.size, t.exchange, t.condition, t.sequence);
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
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight ET</div>
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
<div class="param-desc">Trade condition code (encodes SIP condition flags such as regular sale, odd lot, intermarket sweep)</div>
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
<div class="param-desc">Trade price (<code>f64</code>, decoded at parse time).</div>
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
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34200000, "price": 646.42, "size": 126, "exchange": 4, "condition": 1, "sequence": 1704668},
  {"date": 20260402, "ms_of_day": 34200000, "price": 646.42, "size": 126, "exchange": 73, "condition": 95, "sequence": 1704670},
  {"date": 20260402, "ms_of_day": 34200000, "price": 646.42, "size": 126, "exchange": 43, "condition": 95, "sequence": 1704674}
]
```

> SPY trades on 2026-04-02. Full response contains 887,576 trades. Use the `_stream` variant for large responses.

## Notes

- A single day of AAPL trades can exceed 100,000 rows. Use the Rust `_stream` variant for large responses to avoid holding everything in memory.
- Use `start_time` and `end_time` to limit to regular trading hours (9:30 AM = `34200000`, 4:00 PM = `57600000`).
- The `condition` and `condition_flags` fields encode SIP trade condition codes (e.g., regular sale, odd lot, intermarket sweep).
