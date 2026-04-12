---
title: MCP Server
description: Model Context Protocol server for ThetaDataDx. Gives any LLM instant access to 64 ThetaData tools via JSON-RPC 2.0 over stdio.
---

# MCP Server

Model Context Protocol server for ThetaDataDx. Gives any LLM instant access to 64 ThetaData tools via JSON-RPC 2.0 over stdio.

## Installation

```bash
cargo install thetadatadx-mcp --git https://github.com/userFRM/ThetaDataDx
```

## Configuration

### Claude Code

Add to `.claude/settings.json`:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETA_EMAIL": "you@example.com",
        "THETA_PASSWORD": "your-password"
      }
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETA_EMAIL": "you@example.com",
        "THETA_PASSWORD": "your-password"
      }
    }
  }
}
```

::: warning
Store credentials in environment variables or a secrets manager rather than committing them to config files in version control.
:::

## Available Tools (68)

65 data endpoints + ping + all_greeks + implied_volatility = 68 tools.

| Category | Count |
|----------|-------|
| Meta | 1 (`ping`) |
| Offline Greeks | 2 (`all_greeks`, `implied_volatility`) |
| Stock | 14 |
| Option | 34 |
| Index | 9 |
| Calendar & Rates | 4 |

## Offline Mode

Without credentials, only `ping`, `all_greeks`, and `implied_volatility` are available. This is useful for testing MCP integration or computing Greeks without a ThetaData subscription.

## Wildcard Option Queries

For option tools, MCP uses `"0"` as the wildcard value for `strike` and `expiration`.

- Use a pinned strike like `"strike":"385"` when you want one contract.
- Use `"strike":"0"` when you want a bulk chain-style response with contract identification fields on each row.
- `strike_range` filters a wildcard bulk selection around spot / ATM. It does not fan out a pinned strike into neighboring strikes.

This matches the underlying SDK contract. The current v3 REST surface uses `*` for the same wildcard concept.

## Logging

```bash
RUST_LOG=debug thetadatadx-mcp       # verbose
RUST_LOG=warn thetadatadx-mcp        # quiet
```

::: tip
All logs go to stderr. Stdout is reserved for JSON-RPC communication. This means you can safely redirect logs without interfering with the MCP protocol.
:::
