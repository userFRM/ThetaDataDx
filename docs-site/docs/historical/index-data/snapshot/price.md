---
title: index_snapshot_price
description: Latest price snapshot for one or more indices.
---

# index_snapshot_price

<TierBadge tier="value" />

Get the latest price snapshot for one or more index symbols. Returns the most recent price data as `Vec<PriceTick>`.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_snapshot_price(&["SPX", "NDX"]).await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2}", t.date, t.ms_of_day, t.price_f64());
}
```
```python [Python]
data = tdx.index_snapshot_price(["SPX", "NDX"])
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f}")
```
```go [Go]
data, _ := client.IndexSnapshotPrice([]string{"SPX"})
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f\n", t.Date, t.MsOfDay, t.Price)
}
```
```cpp [C++]
auto data = client.index_snapshot_price({"SPX"});
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f\n", t.date, t.ms_of_day, t.price);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbols</code><span class="param-type">string[]</span><span class="param-badge required">required</span></div>
<div class="param-desc">One or more index symbols</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight</div>
</div>
</div>

## Response

Returns a `Vec<PriceTick>` with price fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">f64</span></div>
<div class="param-desc">Current index price/level</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">u32</span></div>
<div class="param-desc">Milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 57600000, "price": 6899.87},
  {"date": 20260402, "ms_of_day": 57600000, "price": 21256.72},
  {"date": 20260402, "ms_of_day": 57600000, "price": 18.42}
]
```

> Latest price for SPX, NDX, and VIX.

## Notes

- Returns `Vec<PriceTick>` in Rust.
- For OHLC-structured data, use [index_snapshot_ohlc](./ohlc) instead.
