---
title: MCP Server
description: Give any Model Context Protocol client live access to every historical endpoint plus offline Greeks tools.
---

# MCP Server

`thetadatadx-mcp` is a Model Context Protocol server over stdio: any MCP-capable client (Claude Desktop, Cursor, and others) gets a tool per historical endpoint plus the offline Greeks calculators, speaking JSON-RPC 2.0.

## Install

```bash
cargo install thetadatadx-mcp --git https://github.com/userFRM/ThetaDataDx
```

## Configure your client

Most MCP clients read an `mcpServers` block from a project-local or user-level settings file; the shape is the same across clients (for example `.cursor/mcp.json` in Cursor):

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETADATA_EMAIL": "you@example.com",
        "THETADATA_PASSWORD": "your-password"
      }
    }
  }
}
```

To authenticate with an API key instead of email and password, set `THETADATA_API_KEY` in the `env` block (or pass `--api-key <KEY>` in `args`):

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETADATA_API_KEY": "your-api-key"
      }
    }
  }
}
```

The server resolves credentials in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then a `--creds` file (email on line 1, password on line 2). The same names authenticate the SDK, the server, and every binding.

::: warning
Keep credentials in environment variables or a secrets manager — not in config files committed to version control.
:::

## Tools

Every generated historical endpoint plus `ping`, `all_greeks`, and `implied_volatility`. Tool names and parameters match the [reference pages](/reference/) one-to-one, so the model's tool list is the same surface you read here.

Without credentials, the server still starts and serves the offline tools (`ping`, `all_greeks`, `implied_volatility`) — useful for testing the integration or computing Greeks with no subscription.

## Option queries from a model

- Pin one contract with a concrete strike: `"strike":"385"`.
- Use `"strike":"0"` when you want a bulk chain-style response; rows then carry contract-identity fields.
- `strike_range` narrows a bulk selection around the money; it does not fan a pinned strike out to neighbors.

## Troubleshooting

::: details The client lists no tools
Run `thetadatadx-mcp` by hand: the process must start silently and wait on stdin. Anything printed to stdout breaks the JSON-RPC channel — logs go to stderr by design, so a corrupted stdout usually means a wrapper script is echoing.
:::

::: details Only three tools appear
That is offline mode: credentials were missing or rejected. Check `THETADATA_API_KEY`, or `THETADATA_EMAIL` / `THETADATA_PASSWORD`, in the client's `env` block.
:::

::: details Calls fail with permission errors
The account's tier doesn't cover the endpoint — check the tier badge on the matching [reference page](/reference/) against [Subscriptions](/articles/subscriptions).
:::

::: details Debug logging
`RUST_LOG=debug thetadatadx-mcp` (stderr only; stdout stays clean for the protocol).
:::

::: warning
LLM output varies run to run — treat model-generated parameter choices and analysis as drafts to verify, per [Building with AI / LLMs](/articles/ai-llms).
:::
