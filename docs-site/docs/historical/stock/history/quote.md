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
// 1-minute sampled quotes
let quotes: Vec<QuoteTick> = tdx.stock_history_quote("AAPL", "20240315", "60000").await?;

// Every quote change (stream variant for large responses)
tdx.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```
```python [Python]
# 1-minute sampled quotes
quotes = tdx.stock_history_quote("AAPL", "20240315", "60000")

# Every quote change as DataFrame
df = tdx.stock_history_quote_df("AAPL", "20240315", "0")
print(f"{len(df)} quote changes")
```
```go [Go]
// 1-minute sampled quotes
quotes, err := client.StockHistoryQuote("AAPL", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d quotes\n", len(quotes))
```
```cpp [C++]
// 1-minute sampled quotes
auto quotes = client.stock_history_quote("AAPL", "20240315", "60000");
std::cout << quotes.size() << " quotes" << std::endl;
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

- Setting `interval` to `"0"` returns every NBBO change, which can produce hundreds of thousands of rows for active symbols. Use the Rust `_stream` variant for large responses.
- Python users can use the `_df` variant for direct DataFrame output: `tdx.stock_history_quote_df(...)`.
- Shorthand is supported: `"1m"`, `"5m"`, `"1h"`. Milliseconds (`"60000"`, `"300000"`, `"3600000"`) are auto-converted to the nearest valid preset.
