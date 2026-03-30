---
outline: deep
---

# Real-Time Streaming

<LanguageSelector />

ThetaDataDx supports real-time market data streaming over WebSocket connections with automatic reconnection, backpressure handling, and typed message deserialization.

## Architecture

The streaming client maintains a persistent WebSocket connection to ThetaData's streaming endpoint. Messages are deserialized into strongly-typed structs/objects and dispatched to your handlers with minimal latency.

## Features

- **Automatic Reconnection** -- Configurable exponential backoff on disconnects
- **Backpressure Handling** -- Slow consumers are notified, not crashed
- **Typed Messages** -- Every message is deserialized into a concrete type
- **Multiple Subscriptions** -- Subscribe to many symbols on a single connection

## Sections

- [WebSocket Connection](/streaming/websocket) -- Establishing and managing connections
- [Subscribing to Feeds](/streaming/subscribing) -- Quote, trade, and OHLC subscriptions
- [Handling Messages](/streaming/messages) -- Processing incoming data
- [Reconnection](/streaming/reconnection) -- Resilience and error recovery
