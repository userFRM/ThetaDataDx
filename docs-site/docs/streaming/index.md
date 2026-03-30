---
title: Real-Time Streaming
description: Overview of ThetaDataDx real-time streaming via FPSS - architecture, SDK models, and getting started.
---

# Real-Time Streaming

Real-time market data is delivered via ThetaData's FPSS (Feed Protocol Streaming Service) servers. FPSS delivers live quotes, trades, open interest, and OHLC snapshots over a persistent TLS/TCP connection.

## SDK Streaming Models

Each SDK exposes FPSS differently:

| SDK | Model | Details |
|-----|-------|---------|
| **Rust** | Synchronous callback | Events dispatched through an LMAX Disruptor ring buffer. No Tokio on the streaming hot path. |
| **Python** | Polling | `next_event()` returns events as Python dicts. |
| **Go** | Polling | `NextEvent()` returns events as JSON. |
| **C++** | Polling | `next_event()` returns events as JSON strings. RAII handles cleanup automatically. |

## Available Data Streams

| Stream | Description |
|--------|-------------|
| Quotes | Real-time NBBO bid/ask updates |
| Trades | Individual trade executions |
| Open Interest | Current open interest for options |
| OHLCVC | Aggregated OHLC bars with volume and count |
| Full Trades | All trades for an entire security type (e.g., all stocks) |

## Next Steps

1. [Connecting & Subscribing](./connection) - establish a streaming connection and subscribe to data
2. [Handling Events](./events) - process data and control events in your application
3. [Reconnection & Error Handling](./reconnection) - handle disconnects and recover gracefully
