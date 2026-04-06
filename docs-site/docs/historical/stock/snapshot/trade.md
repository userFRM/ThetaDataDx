---
title: Snapshot Trade
description: Latest trade snapshot for one or more stocks.
---

# stock_snapshot_trade

Latest trade snapshot for one or more stocks. Returns the most recent trade execution for each symbol.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_snapshot_trade(&["SPY"]).await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={} exchange={} condition={}",
        t.date, t.ms_of_day, t.price, t.size, t.exchange, t.condition);
}
```
```python [Python]
data = tdx.stock_snapshot_trade(["SPY"])
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f} "
          f"size={t['size']} exchange={t['exchange']} condition={t['condition']}")
```
```go [Go]
data, _ := client.StockSnapshotTrade([]string{"SPY"})
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f size=%d exchange=%d condition=%d\n",
        t.Date, t.MsOfDay, t.Price, t.Size, t.Exchange, t.Condition)
}
```
```cpp [C++]
auto data = client.stock_snapshot_trade({"SPY"});
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d exchange=%d condition=%d\n",
        t.date, t.ms_of_day, t.price, t.size, t.exchange, t.condition);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbols</code><span class="param-type">string[]</span><span class="param-badge required">required</span></div>
<div class="param-desc">One or more ticker symbols</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight ET</div>
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
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `get_price()`, `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 71983113, "price": 255.35, "size": 50, "exchange": 0, "condition": 1},
  {"date": 20260402, "ms_of_day": 71998357, "price": 655.94, "size": 50, "exchange": 0, "condition": 1}
]
```

## Notes

- Accepts multiple symbols in a single call.
- Prices are stored as fixed-point integers. Use the `get_price()` helper to get the decoded float value.
