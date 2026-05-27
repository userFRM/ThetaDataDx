---
title: option_history_trade
description: All trades for an option contract on a given date.
---

# option_history_trade

<TierBadge tier="standard" />

Retrieve all individual trades for an option contract on a given date.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_trade("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} price={:.2} size={} condition={} exchange={}",
        t.date, t.ms_of_day, t.price, t.size, t.condition, t.exchange);
}
```
```python [Python]
data = tdx.option_history_trade("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} price={t.price:.2f} "
          f"size={t.size} condition={t.condition} exchange={t.exchange}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryTrade('SPY', '20260417', '550', 'C', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} price=${t.price} size=${t.size} condition=${t.condition} exchange=${t.exchange}`);
}
```
```cpp [C++]
auto data = client.option_history_trade("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d price=%.2f size=%d condition=%d exchange=%d\n",
        t.date, t.ms_of_day, t.price, t.size, t.condition, t.exchange);
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
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> or <code>YYYY-MM-DD</code> format, or <code>"*"</code> for all expirations</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike price in dollars (e.g. <code>"550"</code> or <code>"17.5"</code>), or <code>"*"</code> for all strikes. Default: <code>"*"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Option side: <code>"call"</code>, <code>"put"</code>, or <code>"both"</code>. SDK also accepts <code>"C"</code>/<code>"P"</code>. Default: <code>"both"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time (inclusive) in <code>HH:MM:SS.SSS</code> ET wall-clock format. Default: <code>"09:30:00"</code>. Legacy millisecond strings are also accepted.</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time (inclusive) in <code>HH:MM:SS.SSS</code> ET wall-clock format. Default: <code>"16:00:00"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration. Filters contracts returned when <code>expiration="*"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Returns <code>n</code> strikes above and below spot price plus one ATM strike (up to <code>2n + 1</code> strikes).</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format. Use with <code>end_date</code> for multi-day requests. The <code>date</code> argument overrides <code>start_date</code>/<code>end_date</code> when present.</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format.</div>
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
<div class="param-header"><code>sequence</code><span class="param-type">int</span></div>
<div class="param-desc">Sequence number</div>
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
  {"date": 20260402, "ms_of_day": 34203497, "price": 98.59, "size": 1, "condition": 130, "exchange": 6}
]
```

> Trades for SPY 2026-04-17 550 call on 2026-04-02. Deep ITM options may have only 1-2 trades per day.

## Notes

- Multi-day requests are limited to one calendar month and must specify an `expiration` value (a single expiration or `"*"`).
- For liquid contracts, this can return hundreds of thousands of rows. In Rust, use the `_stream` variant to process in chunks.
