# SDKs

Multi-language SDKs for ThetaDataDx. All are thin bindings over the shared Rust core; gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming run inside the `thetadatadx` crate. The language binding is the interface surface.

## Overview

| SDK | Install | Historical | Streaming | Greeks | README |
|---|---|---|---|---|---|
| **Python** | `pip install thetadatadx` | Full generated historical surface | `ThetaDataDx` | `all_greeks()`, chainable `.to_polars()` / `.to_pandas()` / `.to_arrow()` | [sdks/python/](python/) |
| **TypeScript/Node.js** | `npm install thetadatadx` | Full generated historical surface | `ThetaDataDx` | `allGreeks()` | [sdks/typescript/](typescript/) |
| **Go** | `go get github.com/userFRM/thetadatadx/sdks/go` | Full generated historical surface | `FpssClient` | via FFI | [sdks/go/](go/) |
| **C++** | CMake `find_library` | Full generated historical surface | `FpssClient` | via FFI | [sdks/cpp/](cpp/) |
| **C FFI** | `cargo build --release -p thetadatadx-ffi` | Full generated historical surface | `TdxUnified` / `TdxFpssHandle` | `tdx_all_greeks` | [ffi/](../ffi/) |

## Architecture

```
                    +-------------------+
                    |   Your Application |
                    +--------+----------+
                             |
         +----------+-------+-------+----------+
         |          |               |           |
    +----v----+  +--v------+  +----v----+  +---v-----+
    |  Python |  |  Node.js|  |   Go   |  |  C++   |
    |  (PyO3) |  | (napi-rs)|  |  (CGo) |  | (C API)|
    +---------+  +---------+  +--------+  +--------+
         |          |               |           |
         +----------+-------+-------+----------+
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
                    | gRPC (tonic)    |
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

The Python SDK uses [PyO3](https://pyo3.rs/) with [Maturin](https://www.maturin.rs/) for direct Rust-to-Python bindings, bypassing the C FFI layer. The TypeScript/Node.js SDK uses [napi-rs](https://napi.rs/) for direct Rust-to-Node.js bindings via a native addon. The Go and C++ SDKs go through the C FFI crate (`thetadatadx-ffi`), which exposes `extern "C"` functions compiled as both a shared library (`cdylib`) and a static archive (`staticlib`).

## Validation Matrix

- Python: wheel builds and import smoke are validated on Linux x64, macOS arm64 (Apple Silicon), and Windows x64. The package targets the CPython stable ABI (`abi3`) with a minimum version of Python 3.9, so one wheel per platform covers Python 3.9+.
- TypeScript/Node.js: pre-built napi-rs addons are shipped for Linux x64 (glibc), macOS arm64 (Apple Silicon), and Windows x64 (MSVC), on Node.js 18+.
- Go: validated on Linux and macOS with the default `target/release` FFI build. Windows is validated with the GNU Rust target (`x86_64-pc-windows-gnu`), because CGo links through MinGW rather than the MSVC import library used by the C++ SDK.
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
from thetadatadx import Credentials, Config, ThetaDataDx, all_greeks

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Historical data
eod = tdx.stock_history_eod("AAPL", "20240101", "20240315")

# Greeks
g = all_greeks(spot=150.0, strike=155.0, rate=0.05,
               div_yield=0.015, tte=45/365, option_price=3.50, right="C")
```

Requires Python 3.9+. Binary wheels target the CPython stable ABI, so one wheel works across supported Python 3.9+ interpreters on the same platform. See [sdks/python/README.md](python/README.md) for full documentation.

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
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

const eod = tdx.stockHistoryEOD('AAPL', '20240101', '20240315');
```

Requires Node.js 18+. See [sdks/typescript/README.md](typescript/README.md) for full documentation.

## Go SDK

**Binding technology:** CGo wrapping the C FFI layer

```bash
# Build the FFI library first
cargo build --release -p thetadatadx-ffi

# Then use the Go module
go get github.com/userFRM/thetadatadx/sdks/go
```

```go
import tdx "github.com/userFRM/thetadatadx/sdks/go"

creds, _ := tdx.NewCredentials("email", "password")
defer creds.Close()
client, _ := tdx.Connect(creds, tdx.ProductionConfig())
defer client.Close()

eod, _ := client.StockHistoryEOD("AAPL", "20240101", "20240315")
```

Requires Go 1.21+ and a C compiler (for CGo). See [sdks/go/README.md](go/README.md) for full documentation.

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

auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());

auto eod = client.stock_history_eod("AAPL", "20240101", "20240315");
```

Requires C++17, CMake 3.16+, and a C compiler. See [sdks/cpp/README.md](cpp/README.md) for full documentation.

## C FFI Layer

The raw C interface that the Go and C++ SDKs are built on. You can also call it directly from any language with C interop (Swift, Zig, Nim, etc.).

```bash
# Build as shared library (.so / .dylib) and static archive (.a)
cargo build --release -p thetadatadx-ffi
```

The library exposes opaque handle types and `extern "C"` functions:

| Category | Functions |
|---|---|
| **Lifecycle** | `tdx_credentials_new`, `tdx_credentials_from_file`, `tdx_credentials_free` |
| **Config** | `tdx_config_production`, `tdx_config_dev`, `tdx_config_free` |
| **Client** | `tdx_client_connect`, `tdx_client_free` |
| **Unified** | `tdx_unified_connect`, `tdx_unified_historical`, `tdx_unified_*`, `tdx_unified_free` |
| **Greeks** | `tdx_all_greeks`, `tdx_implied_volatility` |
| **Standalone FPSS** | 26 functions: `tdx_fpss_connect`, `tdx_fpss_subscribe_quotes`, `tdx_fpss_subscribe_trades`, `tdx_fpss_subscribe_open_interest`, `tdx_fpss_subscribe_option_quotes`, `tdx_fpss_subscribe_option_trades`, `tdx_fpss_subscribe_option_open_interest`, `tdx_fpss_subscribe_full_trades`, `tdx_fpss_subscribe_full_open_interest`, `tdx_fpss_unsubscribe_quotes`, `tdx_fpss_unsubscribe_trades`, `tdx_fpss_unsubscribe_open_interest`, `tdx_fpss_unsubscribe_option_quotes`, `tdx_fpss_unsubscribe_option_trades`, `tdx_fpss_unsubscribe_option_open_interest`, `tdx_fpss_unsubscribe_full_trades`, `tdx_fpss_unsubscribe_full_open_interest`, `tdx_fpss_is_authenticated`, `tdx_fpss_contract_lookup`, `tdx_fpss_contract_map`, `tdx_fpss_active_subscriptions`, `tdx_fpss_next_event`, `tdx_fpss_event_free`, `tdx_fpss_reconnect`, `tdx_fpss_shutdown`, `tdx_fpss_free` |
| **Memory** | `tdx_*_array_free` (per tick type), `tdx_string_array_free`, `tdx_string_free`, `tdx_last_error` |

All historical data endpoints are accessed through `tdx_client_connect`. Streaming can be reached either through the unified handle (`TdxUnified`, one auth/session for historical + streaming) or the standalone FPSS handle (`TdxFpssHandle`). Results are returned as typed `#[repr(C)]` struct arrays (e.g. `TdxEodTickArray`, `TdxOhlcTickArray`) that must be freed with the corresponding `tdx_*_array_free` function. List endpoints return `TdxStringArray`. See the [FFI source](../ffi/src/lib.rs) for the full API and safety contract.

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

# 5. Go SDK - no separate build step; CGo links at compile time
cd sdks/go/examples && go build . && cd ../../..
```
