# Getting Started with Go

## Prerequisites

- Go 1.21+
- Rust toolchain (for building the FFI library)
- C compiler (for CGo)

## Installation

First, build the Rust FFI library:

```bash
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
```

This produces `target/release/libthetadatadx_ffi.so` (Linux) or `libthetadatadx_ffi.dylib` (macOS).

Then add the Go module:

```bash
go get github.com/userFRM/ThetaDataDx/sdks/go
```

## Credentials

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

## First Query

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/ThetaDataDx/sdks/go"
)

func main() {
    // Load credentials
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil {
        log.Fatal(err)
    }
    defer creds.Close()

    // Connect
    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Fetch end-of-day data
    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil {
        log.Fatal(err)
    }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
    }

    // Compute Greeks (offline, no server call)
    g, err := thetadatadx.AllGreeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("IV=%.4f Delta=%.4f Gamma=%.6f\n", g.IV, g.Delta, g.Gamma)
}
```

## Memory Management

All Go SDK objects that wrap FFI handles must be closed when no longer needed:

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()  // frees the Rust-side allocation

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()
```

## What's Next

- [Historical Data](historical.md) -- all endpoints with Go examples
- [Real-Time Streaming](streaming.md) -- FPSS subscribe and NextEvent
- [API Reference](api-reference.md) -- complete type and method listing
