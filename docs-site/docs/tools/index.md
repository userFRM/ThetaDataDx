---
title: Tools
description: Command-line, MCP, and drop-in REST/WebSocket server tools built on top of the ThetaDataDx core.
---

# Tools

Three first-party tools built on top of the ThetaDataDx core. Every one is a thin wrapper — they all speak to the same Rust client and share the same 61 historical endpoints plus FPSS streaming.

## [CLI (`tdx`)](./cli)

Command-line interface for querying ThetaData market data from your terminal. All 61 endpoints, plus offline Greeks and implied volatility. Install with `cargo install thetadatadx-cli`.

Best for: ad-hoc queries, shell pipelines, cron jobs.

## [MCP Server (`thetadatadx-mcp`)](./mcp)

Model Context Protocol server. Exposes ThetaData tools as JSON-RPC 2.0 over stdio so any MCP-capable LLM client (Cursor, and other editors / agents implementing the MCP spec) can query market data directly. Install with `cargo install thetadatadx-mcp`.

Best for: giving LLMs live access to ThetaData without writing an integration layer.

## [REST Server (`thetadatadx-server`)](./server)

Drop-in HTTP/WebSocket replacement for the ThetaData Java Terminal v3 surface. Existing scripts that target the terminal's v3 routes can point at this binary with no code change. Install with `cargo install thetadatadx-server`.

Best for: serving the local v3 REST and WebSocket route surface.
