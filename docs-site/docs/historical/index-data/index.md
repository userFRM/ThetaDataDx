---
title: Index Endpoints
description: 9 index data endpoints - list symbols, snapshots, history, and at-time queries for market indices.
---

# Index Endpoints (9)

Market index data for major indices like SPX, NDX, DJI, VIX, and more. All endpoints follow the same pattern as stock endpoints but operate on index symbols.

## Endpoint Categories

| Category | Endpoints | Description |
|----------|-----------|-------------|
| [List](./list/symbols) | 2 | Available symbols and dates |
| [Snapshot](./snapshot/ohlc) | 3 | Latest OHLC, price, and market value |
| [History](./history/eod) | 3 | EOD, intraday OHLC, and price history |
| [At-Time](./at-time/price) | 1 | Price at a specific time of day |

## Quick Example

::: code-group
```rust [Rust]
let symbols = tdx.index_list_symbols().await?;
let eod = tdx.index_history_eod("SPX", "20240101", "20240301").await?;
```
```python [Python]
symbols = tdx.index_list_symbols()
eod = tdx.index_history_eod("SPX", "20240101", "20240301")
```
```typescript [TypeScript]
const symbols = tdx.indexListSymbols();
const eod = tdx.indexHistoryEOD('SPX', '20240101', '20240301');
```
```go [Go]
symbols, _ := client.IndexListSymbols()
eod, _ := client.IndexHistoryEOD("SPX", "20240101", "20240301")
```
```cpp [C++]
auto symbols = client.index_list_symbols();
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
```
:::

## Common Index Symbols

| Symbol | Description |
|--------|-------------|
| `SPX` | S&P 500 |
| `NDX` | Nasdaq 100 |
| `DJI` | Dow Jones Industrial Average |
| `RUT` | Russell 2000 |
| `VIX` | CBOE Volatility Index |

::: tip
Use `index_list_symbols` to get the full list of available index symbols from ThetaData.
:::
