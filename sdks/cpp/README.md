# thetadatadx (C++)

C++ SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via C FFI.

**This is NOT a C++ reimplementation.** Every call goes through compiled Rust via a C FFI layer. gRPC communication, protobuf parsing, zstd decompression, and TCP streaming all happen at native Rust speed. C++ is just the interface.

## Prerequisites

- C++17 compiler
- CMake 3.16+
- Rust toolchain (for building the FFI library)

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

Then build the C++ SDK:

```bash
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```

Run the example:

```bash
./thetadatadx_example
```

## Quick Start

```cpp
#include "thetadatadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (auto& tick : eod) {
        std::cout << tick.date << ": O=" << tick.open << std::endl;
    }

    // Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
    std::cout << "IV=" << g.iv << " Delta=" << g.delta << std::endl;
}
```

## API

### Credentials
- `Credentials::from_file(path)` -- load from creds.txt
- `Credentials::from_email(email, password)` -- direct construction

### Config
- `Config::production()` -- ThetaData NJ production servers
- `Config::dev()` -- dev servers with shorter timeouts

### Client
RAII class. All methods throw `std::runtime_error` on failure.

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_list_symbols()` | `vector<string>` | All stock symbols |
| `stock_history_eod(symbol, start, end)` | `vector<EodTick>` | EOD data |
| `stock_history_ohlc(symbol, date, interval)` | `vector<OhlcTick>` | Intraday OHLC |
| `stock_history_trade(symbol, date)` | `vector<TradeTick>` | All trades |
| `stock_history_quote(symbol, date, interval)` | `vector<QuoteTick>` | NBBO quotes |
| `stock_snapshot_quote(symbols)` | `vector<QuoteTick>` | Live quote snapshot |
| `option_list_expirations(symbol)` | `vector<string>` | Expiration dates |
| `option_list_strikes(symbol, exp)` | `vector<string>` | Strike prices |
| `option_list_symbols()` | `vector<string>` | Option underlyings |
| `index_list_symbols()` | `vector<string>` | Index symbols |

### Standalone functions
- `all_greeks(spot, strike, rate, div_yield, tte, price, is_call)` -- returns `Greeks` struct with 22 fields
- `implied_volatility(spot, strike, rate, div_yield, tte, price, is_call)` -- returns `pair<double, double>`

## Architecture

```
C++ code
    |  (RAII wrappers)
    v
thetadatadx.h (C FFI)
    |
    v
libthetadatadx_ffi.so / .a
    |  (Rust FFI crate)
    v
thetadatadx Rust crate
    |  (tonic gRPC / tokio TCP)
    v
ThetaData servers
```
