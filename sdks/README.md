# SDKs

Multi-language SDKs for ThetaDataDx. All are thin bindings over the shared Rust core; gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming run inside the `thetadatadx` crate. The language binding is the interface surface.

## Overview

| SDK | Install | Historical | Streaming | Greeks | README |
|---|---|---|---|---|---|
| **Python** | `pip install thetadatadx` | Full generated historical surface | `Client` | `all_greeks()`, chainable `.to_polars()` / `.to_pandas()` / `.to_arrow()` | [sdks/python/](python/) |
| **TypeScript/Node.js** | `npm install thetadatadx` | Full generated historical surface | `Client` | `allGreeks()` | [sdks/typescript/](typescript/) |
| **C++** | CMake `find_library` | Full generated historical surface | `StreamingClient` | via FFI | [sdks/cpp/](cpp/) |
| **C FFI** | `cargo build --release -p thetadatadx-ffi` | Full generated historical surface | `ThetaDataDxClient` / `ThetaDataDxStreamHandle` | `thetadatadx_all_greeks` | [ffi/](../ffi/) |

## Architecture

```
                    +-------------------+
                    |   Your Application |
                    +--------+----------+
                             |
              +----------+-------+----------+
              |          |                  |
         +----v----+  +--v------+      +---v-----+
         |  Python |  |  Node.js|      |   C++   |
         |  (PyO3) |  | (napi-rs)|     | (C API) |
         +---------+  +---------+      +---------+
              |          |                  |
              +----------+-------+----------+
                             |
                    +--------v--------+
                    |   C FFI Layer   |
                    | thetadatadx-ffi |
                    +--------+--------+
                             |
                    +--------v--------+
                    |   Rust Core     |
                    |  thetadatadx    |
                    +-----------------+
                    | gRPC (HTTP/2)       |
                    | Protobuf (prost)|
                    | zstd            |
                    | FPSS (TCP)      |
                    +--------+--------+
                             |
                    +--------v--------+
                    |      tdbe       |
                    | (data format)   |
                    +-----------------+
                    | FIT/FIE codec   |
                    | Greeks (BSM)    |
                    | Price types     |
                    | Tick structs    |
                    | Enums & flags   |
                    +-----------------+
```

The Python SDK uses [PyO3](https://pyo3.rs/) with [Maturin](https://www.maturin.rs/) for direct Rust-to-Python bindings, bypassing the C FFI layer. The TypeScript/Node.js SDK uses [napi-rs](https://napi.rs/) for direct Rust-to-Node.js bindings via a native addon. The C++ SDK goes through the C FFI crate (`thetadatadx-ffi`), which exposes `extern "C"` functions compiled as both a shared library (`cdylib`) and a static archive (`staticlib`).

## Validation Matrix

- Python: wheel builds and import smoke are validated on Linux x64, macOS arm64 (Apple Silicon), and Windows x64. The package targets the CPython stable ABI (`abi3`) with a minimum version of Python 3.12, so one wheel per platform covers Python 3.12+.
- TypeScript/Node.js: pre-built napi-rs addons are shipped for Linux x64 (glibc), macOS arm64 (Apple Silicon), and Windows x64 (MSVC), on Node.js 18+.
- C++: validated with CMake builds on Linux, macOS, and Windows against the generated FFI library.

## Python SDK

**Binding technology:** PyO3 + Maturin (direct Rust-to-Python, no C FFI intermediate)

```bash
# From PyPI
pip install thetadatadx

# With DataFrame support
pip install thetadatadx[pandas]    # pandas
pip install thetadatadx[polars]    # polars
pip install thetadatadx[all]       # both

# From source (requires Rust toolchain and maturin 1.9.4+)
cd sdks/python
pip install "maturin>=1.9.4,<2.0"
maturin develop --release
```

```python
from thetadatadx import Credentials, Config, Client, all_greeks

creds = Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

# Historical data
eod = client.stock_history_eod("AAPL", "20240101", "20240315")

# Greeks
g = all_greeks(spot=150.0, strike=155.0, rate=0.05,
               div_yield=0.015, tte=45/365, option_price=3.50, right="C")
```

Requires Python 3.12+. Binary wheels target the CPython stable ABI, so one wheel works across supported Python 3.12+ interpreters on the same platform. See [sdks/python/README.md](python/README.md) for full documentation.

## TypeScript / Node.js SDK

**Binding technology:** napi-rs native addon (direct Rust-to-Node.js)

```bash
# From npm (once published)
npm install thetadatadx

# From source (requires Rust toolchain)
cd sdks/typescript
npm install
npm run build
```

```typescript
import { Client } from 'thetadatadx';

const client = await Client.connectFromFile('creds.txt');

const eod = client.stockHistoryEOD('AAPL', '20240101', '20240315');
```

Requires Node.js 18+. See [sdks/typescript/README.md](typescript/README.md) for full documentation.

## C++ SDK

**Binding technology:** RAII C++ wrappers around the C FFI header (`thetadx.h`)

```bash
# Build the FFI library first
cargo build --release -p thetadatadx-ffi

# Then build the C++ SDK with CMake
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```

```cpp
#include "thetadx.hpp"

auto creds = thetadatadx::Credentials::from_file("creds.txt");
auto client = thetadatadx::HistoricalClient::connect(creds, thetadatadx::Config::production());

auto eod = client.stock_history_eod("AAPL", "20240101", "20240315");
```

Requires C++17, CMake 3.16+, and a C compiler. See [sdks/cpp/README.md](cpp/README.md) for full documentation.

## C FFI Layer

The raw C interface that the C++ SDK is built on. You can also call it directly from any language with C interop.

```bash
# Build as shared library (.so / .dylib) and static archive (.a)
cargo build --release -p thetadatadx-ffi
```

The library exposes opaque handle types and `extern "C"` functions:

| Category | Functions |
|---|---|
| **Lifecycle** | `thetadatadx_credentials_from_email`, `thetadatadx_credentials_from_file`, `thetadatadx_credentials_free` |
| **Config** | `thetadatadx_config_production`, `thetadatadx_config_dev`, `thetadatadx_config_free` |
| **HistoricalClient** | `thetadatadx_historical_connect`, `thetadatadx_historical_free` |
| **Unified** | `thetadatadx_client_connect`, `thetadatadx_client_historical`, `thetadatadx_client_*`, `thetadatadx_client_free` |
| **Greeks** | `thetadatadx_all_greeks`, `thetadatadx_implied_volatility` |
| **Standalone streaming** | `thetadatadx_streaming_connect`, `thetadatadx_streaming_set_callback`, `thetadatadx_streaming_subscribe`, `thetadatadx_streaming_unsubscribe` (both polymorphic, take `ThetaDataDxSubscriptionRequest`), `thetadatadx_streaming_is_authenticated`, `thetadatadx_streaming_active_subscriptions`, `thetadatadx_streaming_reconnect`, `thetadatadx_streaming_dropped_events`, `thetadatadx_streaming_shutdown`, `thetadatadx_streaming_await_drain`, `thetadatadx_streaming_free` |
| **Memory** | `thetadatadx_*_array_free` (per tick type), `thetadatadx_string_array_free`, `thetadatadx_string_free`, `thetadatadx_last_error` |

All historical data endpoints (61 total) are accessed through `thetadatadx_historical_connect`. Streaming can be reached either through the unified handle (`ThetaDataDxClient`, one auth/session for historical + streaming) or the standalone FPSS handle (`ThetaDataDxStreamHandle`). Results are returned as typed `#[repr(C)]` struct arrays (e.g. `ThetaDataDxEodTickArray`, `ThetaDataDxOhlcTickArray`) that must be freed with the corresponding `thetadatadx_*_array_free` function. List endpoints return `ThetaDataDxStringArray`. See the [FFI source](../ffi/src/lib.rs) for the full API and safety contract.

## Building All SDKs

From the repository root:

```bash
# 1. Build the Rust core and FFI library
cargo build --release -p thetadatadx-ffi

# 2. Build the Python SDK (editable install)
cd sdks/python && maturin develop --release && cd ../..

# 3. Build the TypeScript/Node.js SDK
cd sdks/typescript && npm install && npm run build && cd ../..

# 4. Build the C++ SDK
cmake -S sdks/cpp -B build/cpp
cmake --build build/cpp --config Release --target thetadatadx_cpp
```
