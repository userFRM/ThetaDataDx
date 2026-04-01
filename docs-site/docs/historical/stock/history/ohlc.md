---
title: History OHLC
description: Intraday OHLC bars for a single date or across a date range.
---

# stock_history_ohlc / stock_history_ohlc_range

Intraday OHLC bars at a configurable interval. Two variants are available:

- **stock_history_ohlc** - bars for a single date
- **stock_history_ohlc_range** - bars across a date range

<TierBadge tier="value" />

## Code Example (Single Date)

::: code-group
```rust [Rust]
// 1-minute bars for a single date
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc("AAPL", "20240315", "60000").await?;
println!("{} bars", bars.len());
```
```python [Python]
# 1-minute bars for a single date
bars = tdx.stock_history_ohlc("AAPL", "20240315", "60000")
print(f"{len(bars)} bars")
```
```go [Go]
// 1-minute bars for a single date
bars, err := client.StockHistoryOHLC("AAPL", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d bars\n", len(bars))
```
```cpp [C++]
// 1-minute bars for a single date
auto bars = client.stock_history_ohlc("AAPL", "20240315", "60000");
std::cout << bars.size() << " bars" << std::endl;
```
:::

## Code Example (Date Range)

::: code-group
```rust [Rust]
// 5-minute bars across a date range
let bars: Vec<OhlcTick> = tdx.stock_history_ohlc_range(
    "AAPL", "20240101", "20240301", "300000"
).await?;
```
```python [Python]
# 5-minute bars across a date range
bars = tdx.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000")
```
```go [Go]
// 5-minute bars across a date range
bars, err := client.StockHistoryOHLCRange("AAPL", "20240101", "20240301", "300000")
```
```cpp [C++]
// 5-minute bars across a date range
auto bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000");
```
:::

## Parameters (Single Date)

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
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
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

## Parameters (Date Range)

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
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
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
</div>
</div>

## Response Fields (OhlcTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Bar start time (milliseconds from midnight ET)</div>
</div>
<div class="param">
<div class="param-header"><code>open</code> / <code>high</code> / <code>low</code> / <code>close</code><span class="param-type">i32</span></div>
<div class="param-desc">Fixed-point OHLC prices. Use <code>open_price()</code>, <code>high_price()</code>, <code>low_price()</code>, <code>close_price()</code> for decoded <code>f64</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">i32</span></div>
<div class="param-desc">Total volume in the bar</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">i32</span></div>
<div class="param-desc">Number of trades in the bar</div>
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

## Common Intervals

| Shorthand | Milliseconds |
|-----------|-------------|
| `"1m"` | `"60000"` |
| `"5m"` | `"300000"` |
| `"15m"` | `"900000"` |
| `"1h"` | `"3600000"` |

Milliseconds are auto-converted to the nearest valid preset internally. Either form can be used.

## Notes

- Use the single-date variant for intraday analysis of a specific session.
- Use the range variant for building multi-day bar charts or backtesting.
- Optional `start_time` / `end_time` parameters (single-date variant only) let you filter to regular trading hours or a custom window.
