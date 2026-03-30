---
title: History Trade+Quote
description: Combined trade and prevailing NBBO quote ticks for a stock on a given date.
---

# stock_history_trade_quote

Combined trade + quote ticks for a stock on a given date. Each row contains the full trade data plus the prevailing NBBO quote at the time of the trade.

<TierBadge tier="professional" />

## Code Example

::: code-group
```rust [Rust]
let ticks: Vec<TradeQuoteTick> = tdx.stock_history_trade_quote("AAPL", "20240315").await?;
```
```python [Python]
result = tdx.stock_history_trade_quote("AAPL", "20240315")
```
```go [Go]
result, err := client.StockHistoryTradeQuote("AAPL", "20240315")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto tq = client.stock_history_trade_quote("AAPL", "20240315");
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
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>exclusive</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Use exclusive time bounds</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Response Fields (TradeQuoteTick)

Combined trade + quote tick (25 fields). Contains the full trade data plus the prevailing NBBO quote at the time of the trade.

Helper methods: `trade_price()`, `bid_price()`, `ask_price()`

## Notes

- This endpoint merges trade and quote streams so each trade row includes the best bid/ask at the time of execution. Useful for computing effective spread, price improvement, and trade classification.
- Returns `Vec<TradeQuoteTick>` in Rust. Python returns dicts, Go/C++ return JSON.
- This is a Pro-tier endpoint. Value and Standard subscriptions do not have access.
- The response can be very large for active symbols. Consider filtering with `start_time` / `end_time` or using date ranges that cover only the session you need.
