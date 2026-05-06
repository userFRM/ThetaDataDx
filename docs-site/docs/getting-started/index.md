---
title: Getting Started
description: Multi-language ThetaData SDK in Rust, Python, TypeScript, and C++. Install, authenticate, run a first query, stream, and compute Greeks locally.
---

# Getting Started

ThetaDataDx is a Rust SDK for ThetaData's MDDS (historical, gRPC) and FPSS (real-time, TCP) servers. The data path — gRPC, protobuf parsing, zstd decompression, FIT decoding, Greeks math — runs inside the `thetadatadx` Rust crate and is exposed in four language surfaces: Rust, Python, TypeScript/Node.js, and C++. Go consumers can build a thin cgo wrapper against the unchanged C ABI in [`ffi/`](https://github.com/userFRM/ThetaDataDx/tree/main/ffi).

## What's on this page

- [Quick Start](./quickstart) — install, authenticate, first historical call, first streaming call, tabbed across all four SDKs
- [Installation](./installation) — install for your language
- [Authentication](./authentication) — credentials file, environment variables, token lifecycle
- [First query](./first-query) — one historical call in every language
- [Streaming (FPSS)](./streaming) — SPKI pinning, callback / polling models, lock-free ring, reconnect policy
- [Greeks calculator](./greeks) — 22 local Greeks + IV solver, no server round-trip
- [DataFrames](./dataframes) — Arrow / Polars / Pandas output with the zero-copy scope
- [Error handling](./errors) — `ThetaDataError` hierarchy, retry policy, session refresh

## Prerequisites

| Requirement | Details |
|-------------|---------|
| ThetaData account | Email and password from [thetadata.us](https://thetadata.us) |
| Rust toolchain | Required for the C++ SDK (builds the FFI library); not required for Rust/Python/TypeScript on supported platforms |
| Python 3.9+ | For the Python SDK; pre-built `abi3` wheels provided |
| Node.js 18+ | For the TypeScript/Node.js SDK |
| C++17 compiler + CMake 3.16+ | For the C++ SDK |

::: tip
The Python SDK ships pre-built `abi3` wheels for common platforms. You do not need a Rust toolchain unless you are building from source or using the C++ SDK.
:::

## Subscription tiers

Some endpoints require a paid ThetaData tier. See the [subscription tier matrix](./subscriptions) for which endpoints are available on which plan.

::: tip ThetaData documentation
For ThetaData's documentation on data coverage, exchange fees, and account management, visit [docs.thetadata.us](https://docs.thetadata.us/).
:::
