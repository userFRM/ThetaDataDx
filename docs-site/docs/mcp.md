---
title: MCP Server
description: Give any Model Context Protocol client live access to every market-data endpoint.
---

# MCP Server

`thetadatadx-mcp-server` is a Model Context Protocol server over stdio: any MCP-capable client (Claude Desktop, Cursor, and others) gets a tool per market-data endpoint, speaking JSON-RPC 2.0.

## Configure your client

Most MCP clients read an `mcpServers` block from a project-local or user-level settings file; the shape is the same across clients (for example `.cursor/mcp.json` in Cursor). Point the client at `npx`, which downloads and runs the server on demand — no toolchain to install:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server@next"],
      "env": {
        "THETADATA_API_KEY": "your-api-key"
      }
    }
  }
}
```

`npx -y thetadatadx-mcp-server@next` fetches a prebuilt binary for your platform (Linux and macOS on x64 and arm64, Windows on x64) and runs it; nothing else to install. To authenticate with an email and password instead of an API key, swap the `env` block:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server@next"],
      "env": {
        "THETADATA_EMAIL": "you@example.com",
        "THETADATA_PASSWORD": "your-password"
      }
    }
  }
}
```

The server resolves credentials in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then a `--creds` file (email on line 1, password on line 2). The same names authenticate the SDK, the server, and every binding.

### Rust users: build from source

If you already have a Rust toolchain, install the binary directly and set `"command": "thetadatadx-mcp-server"` instead of the `npx` invocation above:

```bash
cargo install thetadatadx-mcp-server --git https://github.com/userFRM/ThetaDataDx
```

::: warning
Keep credentials in environment variables or a secrets manager — not in config files committed to version control.
:::

## Tools

Every generated market-data endpoint plus `ping`. Tool names and parameters match the [reference pages](/reference/) one-to-one, so the model's tool list is the same surface you read here.

Once connected, the server advertises only the tools your subscription grants. A tool appears when its asset class — stock, options, indices, or interest-rate — is covered by your subscription; a class your plan omits contributes no tools. FREE-tier classes stay listed because FREE grants delayed data. The streaming tools are the exception: they are advertised to any connected account, and `stream_market` refuses one below Pro when it is called, naming the tier it needs. `ping` and the trading calendar are offered to every account. The flat-file tools follow the class in their name, and the generic flat-file request appears for any account holding a stock or option tier, since those are the only classes with flat files. Each market-data tool's description names the subscription it needs. Gating is per asset class; within a subscribed class, a call to an endpoint above your tier still returns the usual permission error.

When credentials are present the connected surface also carries the flat-file tools, advertised by class the way the endpoint tools are: an account with only a stock tier sees the stock ones and the generic request, not the option ones. Each pulls a whole-universe daily blob for a single date, writes it to disk as CSV or JSON Lines, and returns the written path:

- `thetadatadx_flatfile_request`: generic flat-file request for a served `(sec_type, req_type)` pair; an unserved pair is rejected with a typed invalid-parameter error.
- `thetadatadx_flatfile_option_trade_quote`: option trade-quote flat file.
- `thetadatadx_flatfile_option_open_interest`: option open-interest flat file.
- `thetadatadx_flatfile_option_eod`: option end-of-day flat file.
- `thetadatadx_flatfile_stock_trade_quote`: stock trade-quote flat file.
- `thetadatadx_flatfile_stock_eod`: stock end-of-day flat file.

A connected server also holds live subscriptions on your behalf and answers questions about them. The endpoint tools return what the vendor serves at the moment you ask; these answer what happened between two moments, which no sequence of snapshots can reconstruct:

- `stream_read`: one contract, one kind of tick. The rows that arrived since your last read of it, with the age of the newest row returned, how far back the rows held reach, and whether anything is missing from the interval you asked about.
- `stream_prints`: each trade on a contract paired with the quote that stood before it, as the feed delivered them.
- `stream_market`: every trade across the whole option or stock market from one subscription, narrowed and ranked on the vendor's own fields as the prints arrive, with the top rows kept until your next read.
- `stream_list`: what is currently held, whether each buffer is still on the feed, and when each will be released.
- `stream_stop`: close one now.

There are no handles. A buffer is a contract and a kind, opened by the first read of it and closed once fifteen minutes have passed without any of these tools using it, by the next call to one of them rather than on a timer, so nothing is released while the server sits idle. Using is wider than reading: a `stream_prints` call keeps the trade and quote legs it reads from alive without moving either one's window. The first `stream_read`, `stream_prints` or `stream_market` on a contract nothing was watching returns nothing yet and the second returns what arrived in between; a `stream_prints` on a contract `stream_read` already holds for its trades has them straight away, without the quotes beside them. `stream_list` and `stream_stop` answer immediately. Every answer reports the feed's own state alongside the rows, because a dead feed and a quiet contract look identical from an age alone. The rows carry the vendor's condition and exchange codes as they arrived, and the vendor's own bar is served as the vendor sent it, with the age of that bar beside it. Two things on this surface are computed rather than sent, and the tool returning each says so. `spread`, ask minus bid, which `stream_market` can narrow and rank on and which is never a column on a row. And a `market_value` row in full: the vendor sends the underlying quote, and `market_bid`, `market_ask` and `market_price` are derived from it by the size-imbalance rule the vendor's own terminal uses, so they are a theoretical market value and not the quote. A quote or bar carried inside an answer has its own `age_ms` beside it, which is the age of that row and not of the answer.

A stream subscription is scoped to the account, not to the connection. `stream_stop` therefore stops the stream for every application on that account, not only for this server.

The `entitlements` tool reports the subscription tier held for each asset class, which is what decides the rest of the tool list.

Without credentials, the server still starts and serves the offline tool (`ping`) — useful for testing the integration. The flat-file tools, the streaming tools and the market-data endpoints need a live connection.

## Option queries from a model

- Pin one contract with a concrete strike: `"strike":"385"`.
- Use `"strike":"0"` when you want a bulk chain-style response; rows then carry contract-identity fields.
- `strike_range` narrows a bulk selection around the money; it does not fan a pinned strike out to neighbors.

## Troubleshooting

::: details The client lists no tools
Run `thetadatadx-mcp-server` by hand: the process must start silently and wait on stdin. Anything printed to stdout breaks the JSON-RPC channel — logs go to stderr by design, so a corrupted stdout usually means a wrapper script is echoing.
:::

::: details Only `ping` appears
That is offline mode: credentials were missing or rejected. Check `THETADATA_API_KEY`, or `THETADATA_EMAIL` / `THETADATA_PASSWORD`, in the client's `env` block.
:::

::: details Calls fail with permission errors
The account's tier doesn't cover the endpoint — check the tier badge on the matching [reference page](/reference/) against [Subscriptions](/articles/subscriptions).
:::

::: details Debug logging
`RUST_LOG=debug thetadatadx-mcp-server` (stderr only; stdout stays clean for the protocol).
:::

::: warning
LLM output varies run to run — treat model-generated parameter choices and analysis as drafts to verify, per [Building with AI / LLMs](/articles/ai-llms).
:::
