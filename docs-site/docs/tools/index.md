---
outline: deep
---

# Tools

ThetaDataDx ships with three standalone tools that complement the SDK libraries.

## CLI

A command-line interface for quick, ad-hoc data queries. Pipe output to `jq`, CSV files, or your analysis scripts.

```bash
thetadatadx quotes AAPL --date 2025-01-15 --format csv
```

[CLI Documentation](/tools/cli)

## MCP Server

A Model Context Protocol server that exposes ThetaData queries as tools for AI assistants (Claude, GPT, etc.). Run it alongside your AI coding workflow for instant market data access.

```bash
thetadatadx mcp-server --port 3100
```

[MCP Server Documentation](/tools/mcp-server)

## REST Server

A lightweight HTTP proxy that serves ThetaData responses as a standard REST API. Useful for dashboards, web frontends, or any language without a native SDK.

```bash
thetadatadx rest-server --port 8080
```

[REST Server Documentation](/tools/rest-server)
