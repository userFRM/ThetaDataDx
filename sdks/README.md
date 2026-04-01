# SDKs

Multi-language SDKs for ThetaDataDx. All powered by the Rust core via FFI - these are **not** reimplementations. Every SDK call goes through compiled Rust: gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming all happen at native speed. The language binding is just the interface.

## Overview

| SDK | Install | Historical | Streaming | Greeks | README |
|---|---|---|---|---|---|
| **Python** | `pip install thetadatadx` | 61 endpoints | `ThetaDataDx` | `all_greeks()`, `to_polars()` / `to_dataframe()` | [sdks/python/](python/) |
| **Go** | `go get github.com/userFRM/thetadatadx/sdks/go` | 61 endpoints | `FpssClient` | via FFI | [sdks/go/](go/) |
| **C++** | CMake `find_library` | 61 endpoints | `FpssClient` | via FFI | [sdks/cpp/](cpp/) |
| **C FFI** | `cargo build --release -p thetadatadx-ffi` | 61 endpoints | 7 functions | `tdx_all_greeks` | [ffi/](../ffi/) |

## Architecture

```
                    +-------------------+
                    |   Your Application |
                    +--------+----------+
                             |
              +--------------+--------------+
              |              |              |
         +----v----+   +----v----+   +----v----+
         |  Python |   |   Go   |   |  C++   |
         |  (PyO3) |   |  (CGo) |   | (C API)|
         +---------+   +--------+   +--------+
              |              |              |
              +--------------+--------------+
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

The Python SDK uses [PyO3](https://pyo3.rs/) with [Maturin](https://www.maturin.rs/) for direct Rust-to-Python bindings, bypassing the C FFI layer. The Go and C++ SDKs go through the C FFI crate (`thetadatadx-ffi`), which exposes `extern "C"` functions compiled as both a shared library (`cdylib`) and a static archive (`staticlib`).

## Python SDK

**Binding technology:** PyO3 + Maturin (direct Rust-to-Python, no C FFI intermediate)

```bash
# From PyPI
pip install thetadatadx

# With DataFrame support
pip install thetadatadx[pandas]    # pandas
pip install thetadatadx[polars]    # polars
pip install thetadatadx[all]       # both

# From source (requires Rust toolchain)
cd sdks/python
pip install maturin
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
               div_yield=0.015, tte=45/365, option_price=3.50, is_call=True)
```

Requires Python 3.9+. See [sdks/python/README.md](python/README.md) for full documentation.

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
| **Greeks** | `tdx_all_greeks`, `tdx_implied_volatility` |
| **FPSS Streaming** | `tdx_fpss_connect`, `tdx_fpss_subscribe_quotes`, `tdx_fpss_subscribe_trades`, `tdx_fpss_unsubscribe_quotes`, `tdx_fpss_next_event`, `tdx_fpss_shutdown`, `tdx_fpss_free` |
| **Memory** | `tdx_string_free`, `tdx_last_error` |

All historical data endpoints (61 total) are accessed through `tdx_client_connect`. Results are returned as JSON strings (`*mut c_char`) that must be freed with `tdx_string_free`. See the [FFI source](../ffi/src/lib.rs) for the full API and safety contract.

## Building All SDKs

From the repository root:

```bash
# 1. Build the Rust core and FFI library
cargo build --release -p thetadatadx-ffi

# 2. Build the Python SDK (editable install)
cd sdks/python && maturin develop --release && cd ../..

# 3. Build the C++ SDK
cd sdks/cpp && mkdir -p build && cd build && cmake .. && make && cd ../../..

# 4. Go SDK - no separate build step; CGo links at compile time
cd sdks/go/examples && go build . && cd ../../..
```
