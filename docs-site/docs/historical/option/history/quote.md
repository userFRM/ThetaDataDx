---
title: option_history_quote
description: NBBO quotes for an option contract at a given interval.
---

# option_history_quote

<TierBadge tier="value" />

Retrieve NBBO quotes for an option contract, sampled at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_quote("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} ask={:.2} bid_size={} ask_size={}",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size);
}
```
```python [Python]
data = tdx.option_history_quote("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} bid={t['bid']:.2f} "
          f"ask={t['ask']:.2f} bid_size={t['bid_size']} ask_size={t['ask_size']}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryQuote('SPY', '20260417', '550', 'C', '20260315', '60000');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} bid=${t.bid} ask=${t.ask} bid_size=${t.bid_size} ask_size=${t.ask_size}`);
}
```
```go [Go]
data, _ := client.OptionHistoryQuote("SPY", "20260417", "550", "C", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f bid_size=%d ask_size=%d\n",
        t.Date, t.MsOfDay, t.Bid, t.Ask, t.BidSize, t.AskSize)
}
```
```cpp [C++]
auto data = client.option_history_quote("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f bid_size=%d ask_size=%d\n",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size);
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
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>. Use <code>"0"</code> for every quote change.</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
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
  {"date": 20260402, "ms_of_day": 34200000, "bid": 0.00, "ask": 0.00, "bid_size": 0, "ask_size": 0},
  {"date": 20260402, "ms_of_day": 34260000, "bid": 97.94, "ask": 98.90, "bid_size": 1, "ask_size": 1},
  {"date": 20260402, "ms_of_day": 34320000, "bid": 97.05, "ask": 100.60, "bid_size": 1, "ask_size": 54}
]
```

> 1-minute NBBO quotes for SPY 2026-04-17 550 call.

## Notes

- Use `"0"` as the interval to get every quote change (tick-by-tick).
- For liquid contracts with `"0"` interval, the response can be very large. In Rust, use the `_stream` variant.
