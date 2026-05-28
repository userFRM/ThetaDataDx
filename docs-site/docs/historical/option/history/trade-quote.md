---
title: option_history_trade_quote
description: Combined trade and quote ticks for an option contract.
---

# option_history_trade_quote

<TierBadge tier="standard" />

Retrieve combined trade + quote ticks for an option contract on a given date. Each row contains both the trade data and the prevailing quote at the time of the trade.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_trade_quote("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} trade_price={:.2} size={} bid={:.2} ask={:.2} exchange={}",
        t.date, t.ms_of_day, t.trade_price, t.size, t.bid, t.ask, t.exchange);
}
```
```python [Python]
data = tdx.option_history_trade_quote("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} trade_price={t.trade_price:.2f} "
          f"size={t.size} bid={t.bid:.2f} ask={t.ask:.2f} exchange={t.exchange}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryTradeQuote('SPY', '20260417', '550', 'C', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} trade_price=${t.trade_price} size=${t.size} bid=${t.bid} ask=${t.ask}`);
}
```
```cpp [C++]
auto data = client.option_history_trade_quote("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d trade_price=%.2f size=%d bid=%.2f ask=%.2f exchange=%d\n",
        t.date, t.ms_of_day, t.trade_price, t.size, t.bid, t.ask, t.exchange);
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
<div class="param-header"><code>exclusive</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">When <code>true</code>, only quotes with timestamp strictly before the trade timestamp are paired. Default: <code>true</code>.</div>
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
<div class="param-desc">Trade size</div>
</div>
<div class="param">
<div class="param-header"><code>condition</code><span class="param-type">int</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Trade exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>bid_price</code><span class="param-type">float</span></div>
<div class="param-desc">Prevailing bid at time of trade</div>
</div>
<div class="param">
<div class="param-header"><code>bid_size</code><span class="param-type">int</span></div>
<div class="param-desc">Prevailing bid size</div>
</div>
<div class="param">
<div class="param-header"><code>ask_price</code><span class="param-type">float</span></div>
<div class="param-desc">Prevailing ask at time of trade</div>
</div>
<div class="param">
<div class="param-header"><code>ask_size</code><span class="param-type">int</span></div>
<div class="param-desc">Prevailing ask size</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">int</span></div>
<div class="param-desc">Milliseconds from midnight</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34203497, "trade_price": 98.59, "size": 1, "bid": 97.94, "ask": 98.90, "exchange": 6},
  {"date": 20260402, "ms_of_day": 34950122, "trade_price": 99.10, "size": 2, "bid": 98.50, "ask": 99.45, "exchange": 10}
]
```

> Each row pairs the trade with the prevailing NBBO quote at execution time.

## Notes

- Multi-day requests are limited to one calendar month and must specify an `expiration` value (a single expiration or `"*"`).
- Useful for trade classification (e.g., determining if a trade hit the bid or lifted the offer).
