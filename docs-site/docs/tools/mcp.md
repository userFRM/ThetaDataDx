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

## Available Tools (64)

61 data endpoints + ping + all_greeks + implied_volatility = 64 tools.

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

## Logging

```bash
RUST_LOG=debug thetadatadx-mcp       # verbose
RUST_LOG=warn thetadatadx-mcp        # quiet
```

::: tip
All logs go to stderr. Stdout is reserved for JSON-RPC communication. This means you can safely redirect logs without interfering with the MCP protocol.
:::
