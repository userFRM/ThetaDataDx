---
title: index_snapshot_market_value
description: Latest market value snapshot for one or more indices.
---

# index_snapshot_market_value

<TierBadge tier="value" />

Get the latest market value snapshot for one or more index symbols.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_snapshot_market_value(&["SPX"]).await?;
for t in &data {
    println!("date={} market_cap={:.4} shares_outstanding={}",
        t.date, t.market_cap, t.shares_outstanding);
}
```
```python [Python]
data = tdx.index_snapshot_market_value(["SPX"])
for t in data:
    print(f"date={t['date']} "
          f"market_cap={t['market_cap']:.4f} shares_outstanding={t['shares_outstanding']}")
```
```go [Go]
data, _ := client.IndexSnapshotMarketValue([]string{"SPX"})
for _, t := range data {
    fmt.Printf("date=%d market_cap=%.4f shares_outstanding=%d\n",
        t.Date, t.MarketCap, t.SharesOutstanding)
}
```
```cpp [C++]
auto data = client.index_snapshot_market_value({"SPX"});
for (const auto& t : data) {
    printf("date=%d market_cap=%.4f shares_outstanding=%d\n",
        t.date, t.market_cap, t.shares_outstanding);
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

Returns a `Vec<MarketValueTick>` with market value fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>market_value</code><span class="param-type">f64</span></div>
<div class="param-desc">Market capitalization / value</div>
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
  {"date": 20260402, "market_cap": 52140000000000, "shares_outstanding": 0}
]
```

> Market value snapshot for SPX. Index-level market cap represents aggregate constituent values.

## Notes

- Returns `Vec<MarketValueTick>` in Rust.
- Market value represents the total capitalization of the index constituents.
