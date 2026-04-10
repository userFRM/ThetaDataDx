---
title: option_at_time_quote
description: Quote at a specific time of day across a date range for an option contract.
---

# option_at_time_quote

<TierBadge tier="free" />

Retrieve the NBBO quote at a specific time of day across a date range for an option contract. Returns one quote per date, the prevailing quote at the specified time.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_at_time_quote("SPY", "20260417", "550", "C", "20260101", "20260301", "34200000").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} ask={:.2} bid_size={} ask_size={}",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size);
}
```
```python [Python]
data = tdx.option_at_time_quote("SPY", "20260417", "550", "C", "20260101", "20260301", "34200000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} bid={t['bid']:.2f} "
          f"ask={t['ask']:.2f} bid_size={t['bid_size']} ask_size={t['ask_size']}")
```
```go [Go]
data, _ := client.OptionAtTimeQuote("SPY", "20260417", "550", "C", "20260101", "20260301", "34200000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d bid=%.2f ask=%.2f bid_size=%d ask_size=%d\n",
        t.Date, t.MsOfDay, t.Bid, t.Ask, t.BidSize, t.AskSize)
}
```
```cpp [C++]
auto data = client.option_at_time_quote("SPY", "20260417", "550", "C", "20260101", "20260301", "34200000");
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
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>time_of_day</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Milliseconds from midnight ET (e.g. <code>"34200000"</code> = 9:30 AM)</div>
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
  {"date": 20260330, "ms_of_day": 43200000, "bid": 88.50, "ask": 91.69, "bid_size": 10, "ask_size": 7},
  {"date": 20260331, "ms_of_day": 43200000, "bid": 90.69, "ask": 93.67, "bid_size": 4, "ask_size": 10},
  {"date": 20260401, "ms_of_day": 43200000, "bid": 107.99, "ask": 110.66, "bid_size": 10, "ask_size": 34}
]
```

> Quote at 12:00 PM ET for SPY 2026-04-17 550 call. One row per date.

## Notes

- Common time values: `"34200000"` (9:30 AM), `"46800000"` (1:00 PM), `"57600000"` (4:00 PM).
- Useful for building daily spread or mid-price time series at a consistent intraday timestamp.
