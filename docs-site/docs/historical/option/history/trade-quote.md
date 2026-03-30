---
title: option_history_trade_quote
description: Combined trade and quote ticks for an option contract.
---

# option_history_trade_quote

<TierBadge tier="professional" />

Retrieve combined trade + quote ticks for an option contract on a given date. Each row contains both the trade data and the prevailing quote at the time of the trade.

## Code Example

::: code-group
```rust [Rust]
let tq: Vec<TradeQuoteTick> = tdx.option_history_trade_quote(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
tq = tdx.option_history_trade_quote("SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
tq, err := client.OptionHistoryTradeQuote("SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto tq = client.option_history_trade_quote("SPY", "20241220", "500000", "C", "20240315");
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
<div class="param-desc">Strike price as scaled integer</div>
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
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>exclusive</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Use exclusive time bounds</div>
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

## Notes

- Useful for trade classification (e.g., determining if a trade hit the bid or lifted the offer).
