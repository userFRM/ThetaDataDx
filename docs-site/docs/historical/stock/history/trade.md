---
title: History Trade
description: All trades for a stock on a given date.
---

# stock_history_trade

Retrieve every trade execution for a stock on a given date. Returns tick-level data with price, size, exchange, and condition codes.

<TierBadge tier="standard" />

## Code Example

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = tdx.stock_history_trade("AAPL", "20240315").await?;
println!("{} trades", trades.len());

// Stream variant for large responses
tdx.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;
```
```python [Python]
trades = tdx.stock_history_trade("AAPL", "20240315")
print(f"{len(trades)} trades")
```
```go [Go]
trades, err := client.StockHistoryTrade("AAPL", "20240315")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d trades\n", len(trades))
```
```cpp [C++]
auto trades = client.stock_history_trade("AAPL", "20240315");
std::cout << trades.size() << " trades" << std::endl;
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
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Response Fields (TradeTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Milliseconds since midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>sequence</code><span class="param-type">i32</span></div>
<div class="param-desc">Sequence number</div>
</div>
<div class="param">
<div class="param-header"><code>ext_condition1</code> through <code>ext_condition4</code><span class="param-type">i32</span></div>
<div class="param-desc">Extended trade condition codes</div>
</div>
<div class="param">
<div class="param-header"><code>condition</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade condition code (encodes SIP condition flags such as regular sale, odd lot, intermarket sweep)</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">i32</span></div>
<div class="param-desc">Trade size in shares</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">i32</span></div>
<div class="param-desc">Exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">i32</span></div>
<div class="param-desc">Fixed-point price. Use <code>get_price()</code> for decoded <code>f64</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>condition_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Condition flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>price_flags</code><span class="param-type">i32</span></div>
<div class="param-desc">Price flags bitmap</div>
</div>
<div class="param">
<div class="param-header"><code>volume_type</code><span class="param-type">i32</span></div>
<div class="param-desc"><code>0</code> = incremental volume, <code>1</code> = cumulative volume</div>
</div>
<div class="param">
<div class="param-header"><code>records_back</code><span class="param-type">i32</span></div>
<div class="param-desc">Records back count</div>
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

Helper methods: `get_price()`, `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`

## Notes

- A single day of AAPL trades can exceed 100,000 rows. Use the Rust `_stream` variant for large responses to avoid holding everything in memory.
- Use `start_time` and `end_time` to limit to regular trading hours (9:30 AM = `34200000`, 4:00 PM = `57600000`).
- The `condition` and `condition_flags` fields encode SIP trade condition codes (e.g., regular sale, odd lot, intermarket sweep).
