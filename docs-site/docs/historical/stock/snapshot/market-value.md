---
title: Snapshot Market Value
description: Latest market value snapshot for one or more stocks.
---

# stock_snapshot_market_value

Latest market value snapshot for one or more stocks.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_snapshot_market_value(&["SPY"]).await?;
for t in &data {
    println!("date={} market_cap={:.4} shares_outstanding={} enterprise_value={:.4}",
        t.date, t.market_cap, t.shares_outstanding, t.enterprise_value);
}
```
```python [Python]
data = tdx.stock_snapshot_market_value(["SPY"])
for t in data:
    print(f"date={t['date']} market_cap={t['market_cap']:.4f} "
          f"shares_outstanding={t['shares_outstanding']} enterprise_value={t['enterprise_value']:.4f}")
```
```go [Go]
data, _ := client.StockSnapshotMarketValue([]string{"SPY"})
for _, t := range data {
    fmt.Printf("date=%d market_cap=%.4f shares_outstanding=%d enterprise_value=%.4f\n",
        t.Date, t.MarketCap, t.SharesOutstanding, t.EnterpriseValue)
}
```
```cpp [C++]
auto data = client.stock_snapshot_market_value({"SPY"});
for (const auto& t : data) {
    printf("date=%d market_cap=%.4f shares_outstanding=%d enterprise_value=%.4f\n",
        t.date, t.market_cap, t.shares_outstanding, t.enterprise_value);
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
  {"date": 20260402, "market_cap": 3842000000000, "shares_outstanding": 15022100000, "enterprise_value": 3756000000000}
]
```

> Market capitalization, shares outstanding, and enterprise value for each requested symbol.

## Notes

- Accepts multiple symbols in a single call.
- Returns `Vec<MarketValueTick>` in Rust.
