---
title: Snapshot Market Value
description: Latest market value snapshot for one or more stocks.
---

# stock_snapshot_market_value

Latest market value snapshot for one or more stocks.

<TierBadge tier="standard" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_snapshot_market_value(&["SPY"]).await?;
for t in &data {
    println!("date={} market_bid={:.4} market_ask={:.4} market_price={:.4}",
        t.date, t.market_bid, t.market_ask, t.market_price);
}
```
```python [Python]
data = tdx.stock_snapshot_market_value(["SPY"])
for t in data:
    print(f"date={t['date']} market_bid={t['market_bid']:.4f} "
          f"market_ask={t['market_ask']:.4f} market_price={t['market_price']:.4f}")
```
```go [Go]
data, _ := client.StockSnapshotMarketValue([]string{"SPY"})
for _, t := range data {
    fmt.Printf("date=%d market_bid=%.4f market_ask=%.4f market_price=%.4f\n",
        t.Date, t.MarketBid, t.MarketAsk, t.MarketPrice)
}
```
```cpp [C++]
auto data = client.stock_snapshot_market_value({"SPY"});
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

## Response

`Vec<MarketValueTick>` with market value fields. The exact fields depend on the data available for the requested symbols.


### Sample Response

```json
[
  {"date": 20260402, "market_bid": 258.50, "market_ask": 258.55, "market_price": 258.52}
]
```

> Market bid, ask, and price for each requested symbol.

## Notes

- Accepts multiple symbols in a single call.
- Returns an array of MarketValueTick records (typed per SDK).
