---
title: Snapshot Quote
description: Latest NBBO quote snapshot for one or more stocks.
---

# stock_snapshot_quote

Latest NBBO (National Best Bid and Offer) quote snapshot for one or more stocks.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_snapshot_quote(&["SPY", "MSFT", "GOOGL"]).await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} bid_size={} ask={:.2} ask_size={} midpoint={:.2}",
        t.date, t.ms_of_day, t.bid, t.bid_size, t.ask, t.ask_size, t.midpoint);
}
```
```python [Python]
data = tdx.stock_snapshot_quote(["SPY", "MSFT", "GOOGL"])
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} bid={t['bid']:.2f} "
          f"bid_size={t['bid_size']} ask={t['ask']:.2f} ask_size={t['ask_size']} midpoint={t['midpoint']:.2f}")
```
```go [Go]
data, _ := client.StockSnapshotQuote([]string{"SPY", "MSFT", "GOOGL"})
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f bid_size=%d ask=%.2f ask_size=%d midpoint=%.2f\n",
        t.Date, t.MsOfDay, t.Bid, t.BidSize, t.Ask, t.AskSize, t.Midpoint)
}
```
```cpp [C++]
auto data = client.stock_snapshot_quote({"SPY", "MSFT", "GOOGL"});
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d bid=%.2f bid_size=%d ask=%.2f ask_size=%d midpoint=%.2f\n",
        t.date, t.ms_of_day, t.bid, t.bid_size, t.ask, t.ask_size, t.midpoint);
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

## Response Fields (QuoteTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Milliseconds since midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code> / <code>ask_size</code><span class="param-type">i32</span></div>
<div class="param-desc">Quote sizes</div>
</div>
<div class="param">
<div class="param-header"><code>bid_exchange</code> / <code>ask_exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Exchange codes</div>
</div>
<div class="param">
<div class="param-header"><code>bid</code> / <code>ask</code><span class="param-type">i32</span></div>
<div class="param-desc">Bid/ask prices (<code>f64</code>, decoded at parse time). <code>midpoint</code> is pre-computed.</div>
</div>
<div class="param">
<div class="param-header"><code>bid_condition</code> / <code>ask_condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition codes</div>
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
  {"date": 20260402, "ms_of_day": 72000220, "bid": 250.30, "bid_size": 51, "ask": 255.38, "ask_size": 201, "midpoint": 252.84},
  {"date": 20260402, "ms_of_day": 72000000, "bid": 655.61, "bid_size": 200, "ask": 656.41, "ask_size": 200, "midpoint": 656.01}
]
```

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- The NBBO represents the best bid and ask across all exchanges.
- The `midpoint` field is pre-computed from bid and ask.
