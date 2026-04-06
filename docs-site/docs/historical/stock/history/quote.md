---
title: History Quote
description: NBBO quotes for a stock at a configurable sampling interval.
---

# stock_history_quote

NBBO quotes for a stock on a given date, sampled at a configurable interval. Use `"0"` as the interval to get every quote change.

<TierBadge tier="standard" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_history_quote("SPY", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} bid_size={} ask={:.2} ask_size={} midpoint={:.2}",
        t.date, t.ms_of_day, t.bid, t.bid_size, t.ask, t.ask_size, t.midpoint);
}
```
```python [Python]
data = tdx.stock_history_quote("SPY", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} bid={t['bid']:.2f} "
          f"bid_size={t['bid_size']} ask={t['ask']:.2f} ask_size={t['ask_size']} midpoint={t['midpoint']:.2f}")
```
```go [Go]
data, _ := client.StockHistoryQuote("SPY", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f bid_size=%d ask=%.2f ask_size=%d midpoint=%.2f\n",
        t.Date, t.MsOfDay, t.Bid, t.BidSize, t.Ask, t.AskSize, t.Midpoint)
}
```
```cpp [C++]
auto data = client.stock_history_quote("SPY", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d bid=%.2f bid_size=%d ask=%.2f ask_size=%d midpoint=%.2f\n",
        t.date, t.ms_of_day, t.bid, t.bid_size, t.ask, t.ask_size, t.midpoint);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>. Use <code>"0"</code> to get every quote change.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
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
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

Helper methods: `bid_price()`, `ask_price()`, `midpoint_price()`, `midpoint_value()`


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34200000, "bid": 646.41, "bid_size": 1360, "ask": 646.42, "ask_size": 120, "midpoint": 646.41},
  {"date": 20260402, "ms_of_day": 34260000, "bid": 646.85, "bid_size": 440, "ask": 646.87, "ask_size": 480, "midpoint": 646.86},
  {"date": 20260402, "ms_of_day": 34320000, "bid": 647.35, "bid_size": 160, "ask": 647.38, "ask_size": 840, "midpoint": 647.36}
]
```

> SPY 1-minute NBBO quotes on 2026-04-02. Full response contains 391 rows.

## Notes

- Setting `interval` to `"0"` returns every NBBO change, which can produce hundreds of thousands of rows for active symbols. Use the Rust `_stream` variant for large responses.
- Python users can use the `_df` variant for direct DataFrame output: `tdx.stock_history_quote_df(...)`.
- Shorthand is supported: `"1m"`, `"5m"`, `"1h"`. Milliseconds (`"60000"`, `"300000"`, `"3600000"`) are auto-converted to the nearest valid preset.
