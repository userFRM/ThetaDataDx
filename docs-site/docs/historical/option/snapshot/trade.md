---
title: option_snapshot_trade
description: Latest trade snapshot for an option contract.
---

# option_snapshot_trade

<TierBadge tier="standard" />

Get the latest trade snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_trade("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.price, t.size, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_trade("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} price={t['price']:.2f} "
          f"size={t['size']} expiration={t['expiration']} strike={t['strike']:.2f}")
```
```typescript [TypeScript]
const data = tdx.optionSnapshotTrade('SPY', '20260417', '550', 'C');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} price=${t.price} size=${t.size} expiration=${t.expiration} strike=${t.strike}`);
}
```
```go [Go]
data, _ := client.OptionSnapshotTrade("SPY", "20260417", "550", "C")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d price=%.2f size=%d expiration=%d strike=%.2f\n",
        t.Date, t.MsOfDay, t.Price, t.Size, t.Expiration, t.Strike)
}
```
```cpp [C++]
auto data = client.option_snapshot_trade("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.price, t.size, t.expiration, t.strike);
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
<div class="param-desc">Strike price in dollars as a string (e.g. <code>"500"</code> or <code>"17.5"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
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
<div class="param-header"><code>price</code><span class="param-type">float</span></div>
<div class="param-desc">Trade price</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">int</span></div>
<div class="param-desc">Trade size (number of contracts)</div>
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
<div class="param-header"><code>condition</code><span class="param-type">int</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Exchange code</div>
</div>
</div>

### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34203497, "price": 98.59, "size": 1, "expiration": 20260417, "strike": 550.0}
]
```

> Latest trade for SPY 2026-04-17 550 call.

