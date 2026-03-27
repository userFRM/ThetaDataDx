# ThetaDataDx

**No-JVM ThetaData Terminal -- native market data SDK for Rust, Python, Go, and C++.**

[![build](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml/badge.svg)](https://github.com/userFRM/ThetaDataDx/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/thetadatadx.svg)](https://crates.io/crates/thetadatadx)
[![PyPI](https://img.shields.io/pypi/v/thetadatadx)](https://pypi.org/project/thetadatadx)
[![license](https://img.shields.io/github/license/userFRM/ThetaDataDx?color=blue)](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE)

## What is ThetaDataDx?

ThetaDataDx connects directly to ThetaData's upstream servers -- MDDS for historical data and FPSS for real-time streaming -- entirely in native Rust. No JVM terminal process, no local Java dependency, no subprocess management. Your application talks to ThetaData's infrastructure with the same wire protocol their own terminal uses.

A valid [ThetaData](https://thetadata.us) subscription is required.

## 61 Endpoints, 4 Languages

| Category | Endpoints |
|----------|-----------|
| Stock | 14 methods (list, snapshot, history, at-time) |
| Option | 34 methods (list, snapshot, history, Greeks, at-time) |
| Index | 9 methods |
| Rate | 1 method |
| Calendar | 3 methods |
| Streaming | 7 subscription methods (FPSS) |
| Greeks | `all_greeks()` + 20 individual functions |

Every endpoint is available in Rust, Python, Go, and C++. The CLI tool (`tdx`) exposes all endpoints from the command line.

## Choose Your Language

### Rust

The core SDK. Async/await on Tokio, zero-copy tick types with fixed-point `Price` encoding, sync callback-based streaming via LMAX Disruptor ring buffer.

```bash
cargo add thetadatadx
```

[Get started with Rust](rust/getting-started.md)

### Python

Native extension built with PyO3. Every call runs through compiled Rust -- gRPC, protobuf parsing, zstd decompression, FIT decoding all at native speed. Full pandas/polars DataFrame support.

```bash
pip install thetadatadx
```

[Get started with Python](python/getting-started.md)

### Go

CGo bindings to the Rust FFI library. Typed Go structs, automatic memory management via `defer Close()`.

```bash
go get github.com/userFRM/ThetaDataDx/sdks/go
```

[Get started with Go](go/getting-started.md)

### C++

RAII wrappers around the C FFI layer. C++17, CMake, automatic resource cleanup.

```bash
cmake ..
make
```

[Get started with C++](cpp/getting-started.md)

## Architecture

```
Your Application (Rust / Python / Go / C++)
    |
    +-- ThetaDataDx (MDDS) ---> ThetaData gRPC servers (historical)
    |
    +-- ThetaDataDx (FPSS) ---> ThetaData TCP servers (real-time)
    |
    +-- Greeks calculator -----> Local computation (no network)
```

No Java runtime. No JVM terminal process. No subprocess. Direct wire-protocol access.

## Disclaimer

Theta Data, ThetaData, and Theta Terminal are trademarks of Theta Data, Inc. / AxiomX LLC. This project is **not affiliated with, endorsed by, or supported by Theta Data**.

ThetaDataDx is an independent, open-source project provided "as is", without warranty of any kind. See the [LICENSE](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE) for full terms.
