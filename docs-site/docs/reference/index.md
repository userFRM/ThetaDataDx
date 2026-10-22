---
title: API Reference
description: Every endpoint, one page, with signatures and runnable samples in Rust, Python, TypeScript, C++, and HTTP.
---

# API Reference

One page per endpoint. Each page carries the typed signature and a runnable sample in **Rust, Python, TypeScript, C++, and HTTP** — pick your language once in the tab bar and every page follows — plus the parameter table, the response schema, and (where a captured response exists) example output rows.

| Group | Endpoints |
|---|---|
| [Stock](/reference/stock/list/symbols) | List, snapshots, history (EOD / OHLC / trades / quotes / trade-quote), at-time lookups. |
| [Option](/reference/option/list/symbols) | Chain discovery, snapshots, history, Greeks (snapshot, interval, per-trade, EOD), at-time lookups. |
| [Index](/reference/index/list/symbols) | List, snapshots, price history, at-time lookups. |
| [Calendar](/reference/calendar/open-today) | Trading-calendar status by day and year. |
| [Interest Rate](/reference/rate/history/eod) | Rate series history. |
| [Flat Files](/articles/flat-files) | Whole-universe files for one date: option EOD / trade-quote / open interest / quote, stock EOD / trade-quote / quote, index EOD. |

Every endpoint has its own page, the flat files included; [Flat Files](/articles/flat-files) covers pulling and streaming them.

Conventions shared by every endpoint — identifiers, units, timestamps — live in [Symbology & Contract Identity](/articles/symbology). Tier badges on each page map to [Subscriptions](/articles/subscriptions).

Connection setup is one [Getting Started](/articles/getting-started) away; every sample on these pages assumes a connected client named `client` and runs as-is with only `creds.txt` present.
