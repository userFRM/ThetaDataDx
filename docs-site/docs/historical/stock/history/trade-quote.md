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
let data = tdx.stock_history_trade_quote("SPY", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} trade_price={:.2} size={} bid={:.2} ask={:.2} exchange={}",
        t.date, t.ms_of_day, t.trade_price, t.size, t.bid, t.ask, t.exchange);
}
```
```python [Python]
data = tdx.stock_history_trade_quote("SPY", "20260315")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} trade_price={t['trade_price']:.2f} "
          f"size={t['size']} bid={t['bid']:.2f} ask={t['ask']:.2f} exchange={t['exchange']}")
```
```go [Go]
data, _ := client.StockHistoryTradeQuote("SPY", "20260315")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d trade_price=%.2f size=%d bid=%.2f ask=%.2f exchange=%d\n",
        t.Date, t.MsOfDay, t.TradePrice, t.Size, t.Bid, t.Ask, t.Exchange)
}
```
```cpp [C++]
auto data = client.stock_history_trade_quote("SPY", "20260315");
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


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34200015, "trade_price": 646.42, "size": 100, "bid": 646.40, "ask": 646.42, "exchange": 4},
  {"date": 20260402, "ms_of_day": 34200023, "trade_price": 646.43, "size": 200, "bid": 646.41, "ask": 646.43, "exchange": 73},
  {"date": 20260402, "ms_of_day": 34200031, "trade_price": 646.41, "size": 50, "bid": 646.40, "ask": 646.42, "exchange": 12}
]
```

> Each row pairs the trade with the prevailing NBBO quote at execution time.

## Notes

- This endpoint merges trade and quote streams so each trade row includes the best bid/ask at the time of execution. Useful for computing effective spread, price improvement, and trade classification.
- Returns `Vec<TradeQuoteTick>` in Rust, list of dicts in Python, `[]TradeQuoteTick` in Go, `vector<TradeQuoteTick>` in C++.
- This is a Pro-tier endpoint. Value and Standard subscriptions do not have access.
- The response can be very large for active symbols. Consider filtering with `start_time` / `end_time` or using date ranges that cover only the session you need.
