---
title: Getting Started
description: Multi-language ThetaData SDK in Rust, Python, TypeScript, Go, and C++. Install, authenticate, run a first query, stream, compute Greeks, and move from the ThetaData Python SDK.
---

# Getting Started

ThetaDataDx is a direct-wire SDK that connects to ThetaData's MDDS (historical) and FPSS (streaming) servers without a Java terminal in the loop. The entire data path — gRPC, protobuf parsing, zstd decompression, FIT decoding, Greeks math — runs in compiled Rust behind five native SDKs: Rust, Python, TypeScript/Node.js, Go, and C++.

The ThetaData Python SDK (`pip install thetadata`) is Python-only, has no streaming, routes Greeks through the server, and materializes every result through a pure-Python decode loop. ThetaDataDx replaces that decode loop with a Rust core and exposes it to four additional languages.

## What's on this page

- [Installation](./installation) — install for your language
- [Authentication](./authentication) — credentials file, environment variables, token lifecycle
- [First query](./first-query) — one historical call in every language
- [Streaming (FPSS)](./streaming) — SPKI pinning, callback / polling models, lock-free ring, reconnect policy
- [Greeks calculator](./greeks) — 22 local Greeks + IV solver, no server round-trip
- [DataFrames](./dataframes) — Arrow / Polars / Pandas output with the zero-copy scope
- [Error handling](./errors) — `ThetaDataError` hierarchy, retry policy, session refresh
- [Async Python](./async-python) — `asyncio.gather` over the async endpoint methods
- [Performance](./performance) — benchmark headline; full data on the [benchmark page](../performance/benchmark)
- [Migration from the ThetaData Python SDK](./migration) — side-by-side mapping; full migration guide at [migration/from-thetadata-python-sdk](../migration/from-thetadata-python-sdk)

## Per-language quickstarts

Each page keeps one language under ~150 lines: install → auth → one historical call → one streaming call → error handling → where to go next.

- [Rust quickstart](../quickstart/rust)
- [Python quickstart](../quickstart/python)
- [TypeScript quickstart](../quickstart/typescript)
- [Go quickstart](../quickstart/go)
- [C++ quickstart](../quickstart/cpp)

## Prerequisites

| Requirement | Details |
|-------------|---------|
| ThetaData account | Email and password from [thetadata.us](https://thetadata.us) |
| Rust toolchain | Required for Go and C++ SDKs (builds the FFI library); not required for Rust/Python/TypeScript on supported platforms |
| Python 3.9+ | For the Python SDK; pre-built `abi3` wheels provided |
| Node.js 18+ | For the TypeScript/Node.js SDK |
| Go 1.21+ | For the Go SDK; also needs a C compiler for CGo |
| C++17 compiler + CMake 3.16+ | For the C++ SDK |

::: tip
The Python SDK ships pre-built `abi3` wheels for common platforms. You do not need a Rust toolchain unless you are building from source or using the Go/C++ SDKs.
:::

## Subscription tiers

Some endpoints require a paid ThetaData tier. See the [subscription tier matrix](./subscriptions) for which endpoints are available on which plan.

::: tip ThetaData documentation
For ThetaData's documentation on data coverage, exchange fees, and account management, visit [docs.thetadata.us](https://docs.thetadata.us/).
:::
