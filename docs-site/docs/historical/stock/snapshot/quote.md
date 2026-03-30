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
let quotes: Vec<QuoteTick> = tdx.stock_snapshot_quote(&["AAPL", "MSFT", "GOOGL"]).await?;
for q in &quotes {
    println!("bid={} ask={} spread={:.4}",
        q.bid_price(), q.ask_price(),
        q.ask_price() - q.bid_price());
}
```
```python [Python]
quotes = tdx.stock_snapshot_quote(["AAPL", "MSFT", "GOOGL"])
for q in quotes:
    print(f"bid={q['bid']:.2f} ask={q['ask']:.2f}")
```
```go [Go]
quotes, err := client.StockSnapshotQuote([]string{"AAPL", "MSFT", "GOOGL"})
if err != nil {
    log.Fatal(err)
}
for _, q := range quotes {
    fmt.Printf("bid=%.2f ask=%.2f\n", q.Bid, q.Ask)
}
```
```cpp [C++]
auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOGL"});
for (auto& q : quotes) {
    std::cout << "bid=" << q.bid << " ask=" << q.ask << std::endl;
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
<div class="param-desc">Fixed-point prices. Use <code>bid_price()</code>, <code>ask_price()</code>, <code>midpoint_price()</code> for decoded values.</div>
</div>
<div class="param">
<div class="param-header"><code>bid_condition</code> / <code>ask_condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition codes</div>
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

Helper methods: `bid_price()`, `ask_price()`, `midpoint_price()`, `midpoint_value()`

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- The NBBO represents the best bid and ask across all exchanges.
- Use `midpoint_price()` to get the midpoint between bid and ask.
