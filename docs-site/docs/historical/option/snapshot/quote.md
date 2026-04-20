---
title: option_snapshot_quote
description: Latest NBBO quote snapshot for an option contract.
---

# option_snapshot_quote

<TierBadge tier="value" />

Get the latest NBBO (National Best Bid and Offer) quote snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_quote("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} ask={:.2} bid_size={} ask_size={} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_quote("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} bid={t.bid:.2f} ask={t.ask:.2f} "
          f"bid_size={t.bid_size} ask_size={t.ask_size} expiration={t.expiration} strike={t.strike:.2f}")
```
```typescript [TypeScript]
const data = tdx.optionSnapshotQuote('SPY', '20260417', '550', 'C');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} bid=${t.bid} ask=${t.ask} bid_size=${t.bid_size} ask_size=${t.ask_size}`);
}
```
```go [Go]
data, _ := client.OptionSnapshotQuote("SPY", "20260417", "550", "C")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f bid_size=%d ask_size=%d expiration=%d strike=%.2f\n",
        t.Date, t.MsOfDay, t.Bid, t.Ask, t.BidSize, t.AskSize, t.Expiration, t.Strike)
}
```
```cpp [C++]
auto data = client.option_snapshot_quote("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f bid_size=%d ask_size=%d expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size, t.expiration, t.strike);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Strike price in dollars as a string</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>bid_price</code><span class="param-type">float</span></div>
<div class="param-desc">Best bid price</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code><span class="param-type">int</span></div>
<div class="param-desc">Bid size</div>
</div>
<div class="param">
<div class="param-header"><code>ask_price</code><span class="param-type">float</span></div>
<div class="param-desc">Best ask price</div>
</div>
<div class="param">
<div class="param-header"><code>ask_size</code><span class="param-type">int</span></div>
<div class="param-desc">Ask size</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">int</span></div>
<div class="param-desc">Milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>bid_exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Bid exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>ask_exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Ask exchange code</div>
</div>
</div>

### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 58497982, "bid": 105.73, "ask": 108.52, "bid_size": 2, "ask_size": 10, "expiration": 20260417, "strike": 550.0}
]
```

> Latest NBBO quote for SPY 2026-04-17 550 call.

