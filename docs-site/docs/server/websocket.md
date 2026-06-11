---
title: Server — WebSocket Streaming
description: Subscribe to real-time events over the local WebSocket endpoint.
---

# WebSocket Streaming

The server bridges [streaming](/streaming/) onto a local WebSocket at `ws://127.0.0.1:25520/v1/events`. Connect, send one JSON envelope per command, and receive one JSON message per event.

## Subscribe envelope

```json
{
    "msg_type": "STREAM",
    "sec_type": "STOCK",
    "req_type": "QUOTE",
    "add": true,
    "id": 1,
    "contract": {"symbol": "AAPL"}
}
```

| Field | Values |
|---|---|
| `sec_type` | `STOCK`, `OPTION`, `INDEX` |
| `req_type` | `QUOTE`, `TRADE`, `OHLC`, `OPEN_INTEREST`, `FULL_TRADES`, `FULL_OPEN_INTEREST` |
| `add` | `true` subscribes, `false` unsubscribes |
| `id` | Your request id; echoed in the acknowledgement |
| `contract` | Omit for `FULL_*` streams |

Option contracts carry the four-tuple, with the strike in **thousandths of a dollar** (the one wire-format exception to the dollars-everywhere rule — see [Symbology](/articles/symbology)):

```json
{"symbol": "SPY", "expiration": 20250321, "strike": 570000, "right": "C"}
```

`{"msg_type": "STOP", "id": 2}` removes every active stream at once. Each command is acknowledged:

```json
{ "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": 1 } }
```

Invalid commands answer with `"response": "ERROR"` and a message naming the offending field.

## Event messages

Events arrive as JSON with a `header.type` of `QUOTE`, `TRADE`, `OHLC`, or `OPEN_INTEREST`, plus a `STATUS` heartbeat every second. `OHLC` bars flow automatically on any active trade subscription (disable with the server's `--no-ohlcvc` flag).

## Try it

```bash
websocat ws://127.0.0.1:25520/v1/events
{"msg_type": "STREAM", "sec_type": "OPTION", "req_type": "TRADE", "id": 1, "add": true, "contract": {"symbol": "SPY", "expiration": 20250321, "strike": 570000, "right": "C"}}
```

## Limits

- **One client at a time.** A second connection takes over the stream; the first receives a Close frame (code 1000, reason `replaced by a new client connection`). Run one server instance per consumer for multi-client setups.
- Text frames are capped at 4 KiB — far above any legitimate envelope.
- Programmatic consumers should prefer the [SDK streaming surface](/streaming/), which adds typed events, automatic reconnect, and drop monitoring.
