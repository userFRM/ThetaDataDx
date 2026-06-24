---
title: Terminology
description: The dozen terms the rest of this documentation assumes.
---

# Terminology

| Term | Meaning |
|---|---|
| **Symbol / root / underlying** | The unique identifier of a stock, index, or option underlying (e.g. `AAPL`, `SPX`, `SPXW`). The three words are interchangeable. |
| **Contract** | One option, identified by symbol + expiration + strike + right. See [Symbology](/articles/symbology). |
| **Right** | The option side: `C` (call) or `P` (put). |
| **Expiration** | The contract's expiration date, written `YYYYMMDD`. |
| **Strike** | The contract's strike price, in dollars on every SDK surface. |
| **NBBO** | National Best Bid and Offer — the consolidated best quote across exchanges. |
| **OPRA** | Options Price Reporting Authority — the consolidated feed for US option trades and quotes. |
| **SIP** | Securities Information Processor — the consolidated feeds for US equities (CTA and UTP). |
| **EOD** | End-of-day report: one row per trading day with OHLC, volume, count, and the closing quote. |
| **Snapshot** | The latest value (quote, trade, OHLC, …) for the current session, served in real time. |
| **Tick** | One typed row of market data — a trade print, a quote update, a bar. |
| **Tier** | Your ThetaData subscription level per asset class: Free, Value, Standard, or Pro. See [Subscriptions](/articles/subscriptions). |
| **Streaming service** | ThetaData's real-time service, the upstream source behind [Streaming](/streaming/). |
| **Historical service** | ThetaData's market-data delivery service, the upstream source behind historical requests. |
