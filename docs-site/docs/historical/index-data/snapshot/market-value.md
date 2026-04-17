---
title: index_snapshot_market_value
description: Latest market value snapshot for one or more indices.
---

# index_snapshot_market_value

<TierBadge tier="standard" />

Get the latest market value snapshot for one or more index symbols.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_snapshot_market_value(&["SPX"]).await?;
for t in &data {
    println!("date={} market_bid={:.4} market_ask={:.4} market_price={:.4}",
        t.date, t.market_bid, t.market_ask, t.market_price);
}
```
```python [Python]
data = tdx.index_snapshot_market_value(["SPX"])
for t in data:
    print(f"date={t['date']} "
          f"market_bid={t['market_bid']:.4f} market_ask={t['market_ask']:.4f} market_price={t['market_price']:.4f}")
```
```typescript [TypeScript]
const data = tdx.indexSnapshotMarketValue(['SPX']);
for (const t of data) {
    console.log(`date=${t.date} market_bid=${t.market_bid} market_ask=${t.market_ask} market_price=${t.market_price}`);
}
```
```go [Go]
data, _ := client.IndexSnapshotMarketValue([]string{"SPX"})
for _, t := range data {
    fmt.Printf("date=%d market_bid=%.4f market_ask=%.4f market_price=%.4f\n",
        t.Date, t.MarketBid, t.MarketAsk, t.MarketPrice)
}
```
```cpp [C++]
auto data = client.index_snapshot_market_value({"SPX"});
for (const auto& t : data) {
    printf("date=%d market_bid=%.4f market_ask=%.4f market_price=%.4f\n",
        t.date, t.market_bid, t.market_ask, t.market_price);
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

Returns an array of MarketValueTick records with market value fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>market_bid</code><span class="param-type">f64</span></div>
<div class="param-desc">Market bid price</div>
</div>
<div class="param">
<div class="param-header"><code>market_ask</code><span class="param-type">f64</span></div>
<div class="param-desc">Market ask price</div>
</div>
<div class="param">
<div class="param-header"><code>market_price</code><span class="param-type">f64</span></div>
<div class="param-desc">Market price</div>
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
  {"date": 20260402, "market_bid": 5214.50, "market_ask": 5215.00, "market_price": 5214.75}
]
```

> Market value snapshot for SPX.

## Notes

- Returns an array of MarketValueTick records (typed per SDK).
- Market bid/ask/price represent the latest quoted values.
