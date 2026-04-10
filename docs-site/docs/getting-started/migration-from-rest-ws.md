---
title: Migration from REST & WS
description: Translate the current official ThetaData REST and WebSocket API surface to ThetaDataDx across Rust, Python, Go, and C++.
---

# Migration from REST & WebSocket

Use this page when you are moving from the current official ThetaData REST or streaming API to ThetaDataDx. It is translation-focused: route shape, parameter normalization, wildcard behavior, and the places where the SDK surface is intentionally more ergonomic or more explicit.

Official references:

- [Current OpenAPI v3 spec](https://docs.thetadata.us/openapiv3.yaml)
- [Streaming getting started](https://docs.thetadata.us/Streaming/Getting-Started.html)
- [Streaming request verification](https://docs.thetadata.us/Streaming/Verify-Stream-Requests.html)

## Language Naming

Once you know the base endpoint name, the per-language naming is mostly mechanical:

| Surface | Example for `option_history_quote` |
|---------|------------------------------------|
| Rust | `tdx.option_history_quote(...).await?` |
| Python | `tdx.option_history_quote(...)` |
| Go | `client.OptionHistoryQuote(...)` |
| C++ | `client.option_history_quote(...)` |

For streaming, Rust uses `ThetaDataDx` plus `Contract`/`SecType`, Python uses `ThetaDataDx`, and Go/C++ use `FpssClient`.

## Parameter Translation Rules

| Official API shape | ThetaDataDx shape | Notes |
|--------------------|-------------------|-------|
| `expiration=2026-04-17` or `20260417` | `"20260417"` | Use compact `YYYYMMDD` in SDK calls. |
| `expiration=*` | `"0"` where wildcard is supported | Applies to bulk option queries in SDK/MCP/server compatibility surfaces. |
| `strike=500` or `500.000` | `"500"` or `"500.0"` | Strike is a dollar string, not a raw wire integer. |
| `strike=*` | `"0"` where wildcard is supported | Use this for bulk option fan-out in SDK/MCP. |
| `right=call` / `put` / `both` | `"C"` / `"P"` / `"*"` | Historical SDK paths normalize `call`/`put`/`*` internally. |
| `time_of_day=09:30:00` | `"34200000"` | SDK historical at-time endpoints use milliseconds from midnight ET. |
| `symbol=AAPL,MSFT` | `&["AAPL", "MSFT"]`, `["AAPL", "MSFT"]`, `[]string{"AAPL", "MSFT"}`, `{"AAPL", "MSFT"}` | Snapshot endpoints take symbol collections directly. |
| Single REST history route with optional `date` vs `start_date/end_date` | Rust builder or fixed-arity helper, depending on language | Rust exposes the richest direct mapping for optional route variants. |

## Critical Option Bulk Rule

`strike_range` is not a magic fan-out switch by itself.

- A pinned strike such as `strike="500"` returns the pinned contract, even if `strike_range` is present.
- To fan out over nearby strikes, wildcard the strike first.
- In the official REST API that means `strike=*`.
- In ThetaDataDx historical and MCP surfaces that means `strike="0"`.

Examples:

::: code-group
```text [Official REST]
/option/history/greeks/eod?...&expiration=20230120&strike=*&strike_range=5&right=call
```
```rust [Rust]
let rows = tdx
    .option_history_greeks_eod("SPY", "20230120", "0", "C", "20221219", "20221220")
    .strike_range(5)
    .await?;
```
```python [Python]
rows = tdx.option_history_greeks_eod(
    "SPY",
    "20230120",
    "0",
    "C",
    "20221219",
    "20221220",
    strike_range=5,
)
```
```go [Go]
rows, err := client.OptionHistoryGreeksEODWithOptions(
    "SPY",
    "20230120",
    "0",
    "C",
    "20221219",
    "20221220",
    &thetadatadx.EndpointRequestOptions{StrikeRange: thetadatadx.Int32(5)},
)
```
```cpp [C++]
tdx::EndpointRequestOptions options;
options.strike_range = 5;
auto rows = client.option_history_greeks_eod("SPY", "20230120", "0", "C", "20221219", "20221220", options);
```
:::

## Historical Route Mapping

The current official REST route suffix usually maps directly to the SDK endpoint name.

### Stock

| Official route suffix | Base SDK method | Notes |
|-----------------------|-----------------|-------|
| `/stock/list/symbols` | `stock_list_symbols` | Direct 1:1. |
| `/stock/list/dates/{request_type}` | `stock_list_dates` | `request_type` moves from a REST path segment to a method argument. |
| `/stock/snapshot/ohlc` | `stock_snapshot_ohlc` | REST comma list becomes a symbol slice/list/vector. |
| `/stock/snapshot/trade` | `stock_snapshot_trade` | Direct 1:1. |
| `/stock/snapshot/quote` | `stock_snapshot_quote` | Direct 1:1. |
| `/stock/snapshot/market_value` | `stock_snapshot_market_value` | Direct 1:1. |
| `/stock/history/eod` | `stock_history_eod` | Direct 1:1. |
| `/stock/history/ohlc` | `stock_history_ohlc` | Rust can also attach optional range/time filters on the same method. |
| `/stock/history/trade` | `stock_history_trade` | Rust can also attach optional range/time filters on the same method. |
| `/stock/history/quote` | `stock_history_quote` | Rust can also attach optional range/time filters on the same method. |
| `/stock/history/trade_quote` | `stock_history_trade_quote` | Rust can also attach optional range/time filters on the same method. |
| `/stock/at_time/trade` | `stock_at_time_trade` | Convert `HH:MM:SS` to ms-from-midnight ET. |
| `/stock/at_time/quote` | `stock_at_time_quote` | Convert `HH:MM:SS` to ms-from-midnight ET. |

SDK-only convenience:

- `stock_history_ohlc_range` exists in Python, Go, C++, and the server/CLI as a dedicated range helper for the merged official OHLC route.

### Option

| Official route suffix | Base SDK method | Notes |
|-----------------------|-----------------|-------|
| `/option/list/symbols` | `option_list_symbols` | Direct 1:1. |
| `/option/list/dates/{request_type}` | `option_list_dates` | `request_type` moves from a REST path segment to a method argument. |
| `/option/list/expirations` | `option_list_expirations` | Direct 1:1. |
| `/option/list/strikes` | `option_list_strikes` | Returns strike strings in dollars. |
| `/option/list/contracts/{request_type}` | `option_list_contracts` | `request_type` moves from a REST path segment to a method argument. |
| `/option/snapshot/ohlc` | `option_snapshot_ohlc` | Direct 1:1. |
| `/option/snapshot/trade` | `option_snapshot_trade` | Direct 1:1. |
| `/option/snapshot/quote` | `option_snapshot_quote` | Direct 1:1. |
| `/option/snapshot/open_interest` | `option_snapshot_open_interest` | Direct 1:1. |
| `/option/snapshot/market_value` | `option_snapshot_market_value` | Direct 1:1. |
| `/option/snapshot/greeks/implied_volatility` | `option_snapshot_greeks_implied_volatility` | Direct 1:1. |
| `/option/snapshot/greeks/all` | `option_snapshot_greeks_all` | Direct 1:1. |
| `/option/snapshot/greeks/first_order` | `option_snapshot_greeks_first_order` | Direct 1:1. |
| `/option/snapshot/greeks/second_order` | `option_snapshot_greeks_second_order` | Direct 1:1. |
| `/option/snapshot/greeks/third_order` | `option_snapshot_greeks_third_order` | Direct 1:1. |
| `/option/history/eod` | `option_history_eod` | Direct 1:1. |
| `/option/history/ohlc` | `option_history_ohlc` | Rust can also attach optional range/time filters. |
| `/option/history/trade` | `option_history_trade` | Rust can also attach optional range/time filters. |
| `/option/history/quote` | `option_history_quote` | Rust can also attach optional range/time filters. |
| `/option/history/trade_quote` | `option_history_trade_quote` | Rust can also attach optional range/time filters. |
| `/option/history/open_interest` | `option_history_open_interest` | Direct 1:1. |
| `/option/history/greeks/eod` | `option_history_greeks_eod` | Direct 1:1. |
| `/option/history/greeks/all` | `option_history_greeks_all` | Rust can also attach optional range/time filters. |
| `/option/history/greeks/first_order` | `option_history_greeks_first_order` | Rust can also attach optional range/time filters. |
| `/option/history/greeks/second_order` | `option_history_greeks_second_order` | Rust can also attach optional range/time filters. |
| `/option/history/greeks/third_order` | `option_history_greeks_third_order` | Rust can also attach optional range/time filters. |
| `/option/history/greeks/implied_volatility` | `option_history_greeks_implied_volatility` | Rust can also attach optional range/time filters. |
| `/option/history/trade_greeks/all` | `option_history_trade_greeks_all` | Direct 1:1. |
| `/option/history/trade_greeks/first_order` | `option_history_trade_greeks_first_order` | Direct 1:1. |
| `/option/history/trade_greeks/second_order` | `option_history_trade_greeks_second_order` | Direct 1:1. |
| `/option/history/trade_greeks/third_order` | `option_history_trade_greeks_third_order` | Direct 1:1. |
| `/option/history/trade_greeks/implied_volatility` | `option_history_trade_greeks_implied_volatility` | Direct 1:1. |
| `/option/at_time/trade` | `option_at_time_trade` | Convert `HH:MM:SS` to ms-from-midnight ET. |
| `/option/at_time/quote` | `option_at_time_quote` | Convert `HH:MM:SS` to ms-from-midnight ET. |

### Index

| Official route suffix | Base SDK method | Notes |
|-----------------------|-----------------|-------|
| `/index/list/symbols` | `index_list_symbols` | Direct 1:1. |
| `/index/list/dates` | `index_list_dates` | Direct 1:1. |
| `/index/snapshot/ohlc` | `index_snapshot_ohlc` | REST comma list becomes a symbol slice/list/vector. |
| `/index/snapshot/price` | `index_snapshot_price` | REST comma list becomes a symbol slice/list/vector. |
| `/index/snapshot/market_value` | `index_snapshot_market_value` | REST comma list becomes a symbol slice/list/vector. |
| `/index/history/eod` | `index_history_eod` | Direct 1:1. |
| `/index/history/ohlc` | `index_history_ohlc` | The SDK exposes start/end dates directly. |
| `/index/history/price` | `index_history_price` | Direct 1:1. |
| `/index/at_time/price` | `index_at_time_price` | Convert `HH:MM:SS` to ms-from-midnight ET. |

### Calendar and Rates

| Official route suffix | Base SDK method | Notes |
|-----------------------|-----------------|-------|
| `/calendar/today` | `calendar_open_today` | Same operation, different naming. |
| `/calendar/on_date` | `calendar_on_date` | Direct 1:1. |
| `/calendar/year_holidays` | `calendar_year` | Same underlying operation; SDK name is calendar-focused. |

SDK-only endpoint:

- `interest_rate_history_eod` is available in ThetaDataDx even though the current official OpenAPI spec does not expose an interest-rate REST route.

## Streaming Request Mapping

### Stocks

| Official request | Rust | Python | Go | C++ |
|------------------|------|--------|----|-----|
| `STREAM` `STOCK` `QUOTE` | `subscribe_quotes(&Contract::stock("AAPL"))` | `subscribe_quotes("AAPL")` | `SubscribeQuotes("AAPL")` | `subscribe_quotes("AAPL")` |
| `STREAM` `STOCK` `TRADE` | `subscribe_trades(&Contract::stock("AAPL"))` | `subscribe_trades("AAPL")` | `SubscribeTrades("AAPL")` | `subscribe_trades("AAPL")` |
| `STREAM` `STOCK` `OPEN_INTEREST` | `subscribe_open_interest(&Contract::stock("AAPL"))` | `subscribe_open_interest("AAPL")` | `SubscribeOpenInterest("AAPL")` | `subscribe_open_interest("AAPL")` |
| `STREAM_BULK` `STOCK` `TRADE` | `subscribe_full_trades(SecType::Stock)` | `subscribe_full_trades("STOCK")` | `SubscribeFullTrades("STOCK")` | `subscribe_full_trades("STOCK")` |

### Options

The official streaming contract still exposes raw terminal-style contract fields:

- `expiration` as an integer date
- `strike` as a raw wire integer in 1/10th-of-a-cent units
- `right` as `C`/`P`

ThetaDataDx intentionally hides that encoding on the user-facing side.

| Official request | Rust | Python | Go | C++ |
|------------------|------|--------|----|-----|
| `STREAM` `OPTION` `QUOTE` | `subscribe_quotes(&Contract::option("SPY", "20260417", "500", "C"))` | `subscribe_option_quotes("SPY", "20260417", "C", "500")` | High-level option convenience wrapper not exposed yet | High-level option convenience wrapper not exposed yet |
| `STREAM` `OPTION` `TRADE` | `subscribe_trades(&Contract::option("SPY", "20260417", "500", "C"))` | `subscribe_option_trades("SPY", "20260417", "C", "500")` | High-level option convenience wrapper not exposed yet | High-level option convenience wrapper not exposed yet |
| `STREAM` `OPTION` `OPEN_INTEREST` | `subscribe_open_interest(&Contract::option("SPY", "20260417", "500", "C"))` | `subscribe_option_open_interest("SPY", "20260417", "C", "500")` | High-level option convenience wrapper not exposed yet | High-level option convenience wrapper not exposed yet |
| `STREAM_BULK` `OPTION` `TRADE` | `subscribe_full_trades(SecType::Option)` | `subscribe_full_trades("OPTION")` | `SubscribeFullTrades("OPTION")` | `subscribe_full_trades("OPTION")` |

### Indices

The official streaming API exposes `STREAM` requests with `sec_type: "INDEX"` and `req_type: "PRICE"`.

- The Rust core has index contract support.
- The public cross-language docs do not yet expose a first-class, ergonomic index-price subscription surface comparable to the stock and option helpers above.
- When you need the current fully documented migration path across all languages, use the historical index endpoints and the documented stock/option streaming helpers.

## Rust Builder vs Fixed-Arity Bindings

Rust is the closest direct projection of the merged current REST routes because required parameters stay in the constructor call and optional query parameters stay on builder methods. That chained style is intentional, not a special case for `strike_range`.

Other SDKs project the same optional surface idiomatically for their language:

- Python uses keyword-only optional parameters
- Go uses `WithOptions` helpers plus `EndpointRequestOptions`
- C++ uses `EndpointRequestOptions` overloads

Example:

```rust
let bars = tdx
    .stock_history_ohlc("AAPL", "20240315", "1m")
    .start_date("20240301")
    .end_date("20240315")
    .start_time("09:30:00")
    .end_time("16:00:00")
    .venue("nqb")
    .await?;
```

Python, Go, and C++ still expose fixed-arity convenience methods for the common path, but selected routes now surface optional builder-style parameters directly as keyword arguments or options structs. When you are translating a merged official REST route, the direct SDK mapping is:

- exact single-date helper when the binding exposes it
- dedicated range helper when the binding exposes it, such as `stock_history_ohlc_range`
- Python keyword-only optional parameters, Go `WithOptions` helpers, or C++ `EndpointRequestOptions` overloads when the binding exposes them
- Rust for the most faithful 1:1 projection of the official optional query surface

## Worked Example

Official REST:

```text
GET /option/at_time/quote?symbol=SPY&expiration=2026-04-17&strike=500&right=call&start_date=2026-04-01&end_date=2026-04-10&time_of_day=09:30:00
```

ThetaDataDx:

::: code-group
```rust [Rust]
let quotes = tdx.option_at_time_quote(
    "SPY",
    "20260417",
    "500",
    "C",
    "20260401",
    "20260410",
    "34200000",
).await?;
```
```python [Python]
quotes = tdx.option_at_time_quote(
    "SPY",
    "20260417",
    "500",
    "C",
    "20260401",
    "20260410",
    "34200000",
)
```
```go [Go]
quotes, err := client.OptionAtTimeQuote(
    "SPY",
    "20260417",
    "500",
    "C",
    "20260401",
    "20260410",
    "34200000",
)
```
```cpp [C++]
auto quotes = client.option_at_time_quote(
    "SPY",
    "20260417",
    "500",
    "C",
    "20260401",
    "20260410",
    "34200000"
);
```
:::
