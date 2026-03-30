---
title: Getting Started
description: Overview of ThetaDataDx -- a direct-wire SDK for ThetaData market data across Rust, Python, Go, and C++.
---

# Getting Started

ThetaDataDx is a multi-language SDK that connects directly to ThetaData's MDDS and FPSS servers. All data processing -- gRPC, protobuf parsing, zstd decompression, and FIT decoding -- runs in compiled Rust regardless of which language you use.

## What You Get

- **61 historical endpoints** covering stocks, options, indices, interest rates, and market calendars
- **Real-time streaming** via FPSS with quote, trade, open interest, and OHLC events
- **Offline Greeks** computed locally with no server round-trip
- **Four SDKs**: Rust (native), Python (PyO3 bindings), Go (CGo FFI), C++ (C FFI with RAII wrappers)

## Prerequisites

| Requirement | Details |
|-------------|---------|
| ThetaData account | Email and password from [thetadata.us](https://thetadata.us) |
| Rust toolchain | Required for Go and C++ SDKs (builds the FFI library) |
| Python 3.9+ | For the Python SDK; pre-built wheels provided |
| Go 1.21+ | For the Go SDK; also needs a C compiler for CGo |
| C++17 compiler + CMake 3.16+ | For the C++ SDK |

::: tip
The Python SDK ships pre-built wheels for common platforms. You do not need a Rust toolchain unless you are building from source or using the Go/C++ SDKs.
:::

## Next Steps

1. [Installation](./installation) -- install the SDK for your language
2. [Authentication](./authentication) -- set up credentials
3. [Quick Start](./quickstart) -- run your first query
