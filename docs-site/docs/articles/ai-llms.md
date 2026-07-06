---
title: Building with AI / LLMs
description: Machine-readable entry points for coding assistants and LLM agents.
---

# Building with AI / LLMs

Three artifacts make this SDK legible to coding assistants and agents:

## llms.txt

[`/llms.txt`](/llms.txt) is a one-line-per-page index of this entire site — path plus a one-sentence summary — regenerated with the documentation. Point a coding assistant at it to let the model pick the right page instead of crawling.

## MCP server

The [MCP server](/mcp) exposes every market-data endpoint to any Model Context Protocol client over JSON-RPC, so an LLM can pull real data mid-conversation. Setup is one config block; see the [MCP page](/mcp).

## OpenAPI specification

[`/thetadatadx.yaml`](/thetadatadx.yaml) describes the [local server](/server/)'s HTTP surface — every route, parameter, and response schema — in OpenAPI 3. Feed it to schema-aware tooling or code generators.

::: warning
LLM output varies run to run. Treat generated queries and generated analysis as drafts: check parameters against the [reference pages](/reference/) and validate results before acting on them.
:::
