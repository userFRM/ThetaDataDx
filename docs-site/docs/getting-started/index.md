---
title: Getting Started
description: Overview of ThetaDataDx - a direct-wire SDK for ThetaData market data across Rust, Python, TypeScript/Node.js, Go, and C++.
---

# Getting Started

ThetaDataDx is a multi-language SDK that connects directly to ThetaData's MDDS and FPSS servers. All data processing - gRPC, protobuf parsing, zstd decompression, and FIT decoding - runs in compiled Rust regardless of which language you use.

## What You Get

- **61 historical endpoints** covering stocks, options, indices, interest rates, and market calendars
- **Real-time streaming** via FPSS with quote, trade, open interest, and OHLC events
- **Offline Greeks** computed locally with no server round-trip
- **Five SDKs**: Rust (native), Python (PyO3 bindings), TypeScript/Node.js (napi-rs), Go (CGo FFI), C++ (C FFI with RAII wrappers)

## Prerequisites

| Requirement | Details |
|-------------|---------|
| ThetaData account | Email and password from [thetadata.us](https://thetadata.us) |
| Rust toolchain | Required for Go and C++ SDKs (builds the FFI library) |
| Python 3.9+ | For the Python SDK; pre-built wheels provided |
| Node.js 18+ | For the TypeScript/Node.js SDK |
| Go 1.21+ | For the Go SDK; also needs a C compiler for CGo |
| C++17 compiler + CMake 3.16+ | For the C++ SDK |

::: tip
The Python SDK ships pre-built wheels for common platforms. You do not need a Rust toolchain unless you are building from source or using the Go/C++ SDKs.
:::

## Next Steps

1. [Subscription Tiers](./subscriptions) - understand what data is available on your plan
2. [Installation](./installation) - install the SDK for your language
3. [Authentication](./authentication) - set up credentials
4. [Quick Start](./quickstart) - run your first query

::: tip ThetaData Documentation
For ThetaData's documentation on data coverage, exchange details, and account management, visit [docs.thetadata.us](https://docs.thetadata.us/).
:::
