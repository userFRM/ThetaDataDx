# thetadatadx (Go)

Go SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via CGo FFI.

**This is NOT a Go reimplementation.** Every call goes through compiled Rust via a C FFI layer. gRPC communication, protobuf parsing, zstd decompression, and TCP streaming all happen at native Rust speed. Go is just the interface.

## Prerequisites

- Go 1.21+
- Rust toolchain (for building the FFI library)
- C compiler (for CGo)

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

This produces `target/release/libthetadatadx_ffi.so` (Linux) or `libthetadatadx_ffi.dylib` (macOS).

Then build or run your Go code:

```bash
cd sdks/go/examples
go run main.go
```

## Quick Start

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/ThetaDataDx/sdks/go"
)

func main() {
    creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, _ := thetadatadx.Connect(creds, config)
    defer client.Close()

    eod, _ := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
    }
}
```

## API

### Credentials
- `NewCredentials(email, password)` -- direct construction
- `CredentialsFromFile(path)` -- load from creds.txt

### Config
- `ProductionConfig()` -- ThetaData NJ production servers
- `DevConfig()` -- dev servers with shorter timeouts

### Client
All data methods return typed Go structs (deserialized from JSON over FFI).

| Method | Returns | Description |
|--------|---------|-------------|
| `StockListSymbols()` | `[]string` | All stock symbols |
| `StockHistoryEOD(symbol, start, end)` | `[]EodTick` | EOD data |
| `StockHistoryOHLC(symbol, date, interval)` | `[]OhlcTick` | Intraday OHLC |
| `StockHistoryTrade(symbol, date)` | `[]TradeTick` | All trades |
| `StockHistoryQuote(symbol, date, interval)` | `[]QuoteTick` | NBBO quotes |
| `StockSnapshotQuote(symbols)` | `[]QuoteTick` | Live quote snapshot |
| `OptionListExpirations(symbol)` | `[]string` | Expiration dates |
| `OptionListStrikes(symbol, exp)` | `[]string` | Strike prices |
| `OptionListSymbols()` | `[]string` | Option underlyings |
| `IndexListSymbols()` | `[]string` | Index symbols |

### Greeks (standalone functions)
- `AllGreeks(spot, strike, rate, divYield, tte, price, isCall)` -- returns `*Greeks` with 22 fields
- `ImpliedVolatility(spot, strike, rate, divYield, tte, price, isCall)` -- returns `(iv, error, err)`

## Architecture

```
Go code
    |  (CGo FFI)
    v
libthetadatadx_ffi.so / .a
    |  (Rust FFI crate)
    v
thetadatadx Rust crate
    |  (tonic gRPC / tokio TCP)
    v
ThetaData servers
```
