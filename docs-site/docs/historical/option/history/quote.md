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
let data = tdx.option_history_quote("SPY", "20260417", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} bid={:.2} ask={:.2} bid_size={} ask_size={}",
        t.date, t.ms_of_day, t.bid, t.ask, t.bid_size, t.ask_size);
}
```
```python [Python]
data = tdx.option_history_quote("SPY", "20260417", "20260315")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} bid={t.bid:.2f} "
          f"ask={t.ask:.2f} bid_size={t.bid_size} ask_size={t.ask_size}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryQuote('SPY', '20260417', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} bid=${t.bid} ask=${t.ask} bid_size=${t.bid_size} ask_size=${t.ask_size}`);
}
```
```cpp [C++]
auto data = client.option_history_quote("SPY", "20260417", "20260315");
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
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Sampling interval. Allowed values: <code>tick</code>, <code>10ms</code>, <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>. Millisecond strings (e.g. <code>"60000"</code>) are accepted and snapped to the nearest preset. Default: <code>"1s"</code>. Sub-minute intervals are available only for single-day requests.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time (inclusive) in <code>HH:MM:SS.SSS</code> ET wall-clock format. Default: <code>"09:30:00"</code>. Legacy millisecond strings (e.g. <code>"34200000"</code>) are also accepted.</div>
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

> 1-second NBBO quotes for SPY 2026-04-17. With wildcard `strike="*"`, every chain strike is returned at the chosen interval.

## Notes

- Multi-day requests are limited to one calendar month and must specify an `expiration` value (a single expiration or `"*"`).
- Wildcard requests (`expiration="*"`, `strike="*"`) can return very large responses. In Rust, use the `_stream` variant to process chunk-by-chunk without buffering the full result.
- The smallest `interval` value upstream accepts is `tick`. Values below `1m` (`tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`) are accepted only for single-day requests.
