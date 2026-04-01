---
title: index_history_price
description: Intraday price history for an index.
---

# index_history_price

<TierBadge tier="standard" />

Retrieve intraday price history for an index on a single date at a specified interval. Returns price data as `Vec<PriceTick>`.

## Code Example

::: code-group
```rust [Rust]
let ticks: Vec<PriceTick> = tdx.index_history_price("SPX", "20240315", "60000").await?;
```
```python [Python]
price = tdx.index_history_price("SPX", "20240315", "60000")
```
```go [Go]
priceHist, err := client.IndexHistoryPrice("SPX", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto price_hist = client.index_history_price("SPX", "20240315", "60000");
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Index symbol (e.g. <code>"SPX"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time of day as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time of day as milliseconds from midnight</div>
</div>
</div>

## Response

Returns a `Vec<PriceTick>` with price and time fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">f64</span></div>
<div class="param-desc">Index price/level</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">u32</span></div>
<div class="param-desc">Milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

## Notes

- Returns `Vec<PriceTick>` in Rust.
- For OHLC-structured data across a date range, use [index_history_ohlc](./ohlc) instead.
- Operates on a single date only. For multi-day queries, use [index_history_eod](./eod) or [index_history_ohlc](./ohlc).
