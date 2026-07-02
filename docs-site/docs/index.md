---
layout: home
title: ThetaDataDx
description: Every ThetaData endpoint, historical, streaming, and bulk, over REST, four native SDKs, or an MCP server for AI.

hero:
  name: "ThetaDataDx"
  text: "Every ThetaData endpoint, however you work."
  tagline: "Historical, streaming, and bulk market data through whatever fits how you build: a REST API, four native SDKs, or an MCP server for your AI."
  actions:
    - theme: brand
      text: Quickstart
      link: /articles/getting-started
    - theme: alt
      text: Browse the REST API
      link: /reference/
    - theme: alt
      text: View on GitHub
      link: https://github.com/userFRM/ThetaDataDx

features:
  - icon:
      src: /icons/globe.svg
    title: "REST API"
    details: "Every endpoint over plain HTTP. Curl it from the shell or call it from any stack. Typed JSON, CSV, or NDJSON responses."
    link: /reference/
  - icon:
      src: /icons/bolt.svg
    title: "Streaming"
    details: "Live quotes, trades, and open interest over WebSocket, delivered as typed events through a callback you register once."
    link: /streaming/
  - icon:
      src: /icons/terminal.svg
    title: "MCP for AI"
    details: "Point Claude or any LLM at ThetaData and ask for an options chain in plain English. A tool per endpoint, no code to write."
    link: /mcp
  - icon:
      src: /icons/rust.svg
    title: "Native SDKs"
    details: "Rust, Python, TypeScript, and C++ over one identical surface. Zero-copy decode, the same method names in every language."
    link: /reference/
  - icon:
      src: /icons/chart.svg
    title: "Self-host Server"
    details: "Run the HTTP + WebSocket gateway yourself on the v3 route surface. Existing scripts point at it unchanged."
    link: /server/
---

## A quote in seconds

The latest NBBO quote for `AAPL`, four ways. Pick the one that fits your stack. Same data, same fields, from a [free or paid subscription](/articles/subscriptions).

::: code-group

```bash [cURL]
# Start the server once: thetadatadx-server --api-key "$THETADATA_API_KEY" &
curl 'http://127.0.0.1:25503/v3/stock/snapshot/quote?symbol=AAPL'
```

```python [Python]
from thetadatadx import Client

client = Client(api_key="your_api_key")
[q] = client.historical.stock_snapshot_quote(["AAPL"])
print(q.bid, q.ask)
```

```typescript [TypeScript]
import { Client } from 'thetadatadx';

const client = await Client.connectWith({ apiKey: 'your_api_key' });
const [q] = await client.historical.stockSnapshotQuote(['AAPL']);
console.log(q.bid, q.ask);
```

```rust [Rust]
let client = thetadatadx::Client::builder().api_key("your_api_key").connect().await?;
let rows = client.historical().stock_snapshot_quote(&["AAPL"]).await?;
println!("{} {}", rows[0].bid, rows[0].ask);
```

:::

## What are you building?

| Goal | Start here |
|---|---|
| A research notebook | [Python SDK](/reference/): install, authenticate, pull history into pandas or polars. |
| A live signal or dashboard | [Streaming](/streaming/): typed quote, trade, and open-interest events over WebSocket. |
| An AI assistant or agent | [MCP for AI](/mcp): give any LLM client a tool per endpoint, no code. |
| A backend or existing tool | [REST API](/reference/) or the [self-host server](/server/): plain HTTP on the v3 routes. |

## Install

The active release line is the **13.0.0 release candidate**. It carries the latest data coverage and fixes, and we recommend installing it. Grab the newest RC:

```bash
pip install --pre thetadatadx          # Python 3.12+ (pinned: pip install thetadatadx==13.0.0rc13)
npm install thetadatadx@next           # Node.js 20+ (pinned: npm install thetadatadx@13.0.0-rc.13)
cargo add thetadatadx@13.0.0-rc.13     # Rust, async over tokio
```

C++ links the same C ABI: build `thetadatadx-ffi`, then include `thetadatadx-cpp/include/thetadatadx.hpp`. Full steps in the [Quickstart](/articles/getting-started).
