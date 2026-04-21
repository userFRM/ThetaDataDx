---
title: Go Quickstart
description: Install, authenticate, run a historical call, subscribe to streaming, and handle errors with ThetaDataDx in Go.
---

# Go Quickstart

CGo bindings around the Rust FFI core. Handles are `Close()`-managed; follow the defer pattern and memory management is trivial.

## Install

```bash
# Prerequisites: Go 1.21+, Rust toolchain, C compiler

# Build the FFI library once:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.{so|dylib}

# On Windows, use the GNU Rust target:
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi

# Then:
go get github.com/userFRM/thetadatadx/sdks/go
```

Set `LD_LIBRARY_PATH` (Linux) or `DYLD_LIBRARY_PATH` (macOS) to the `target/release` directory, or install the `.so` / `.dylib` to a system location.

## Authenticate and connect

```go
import thetadatadx "github.com/userFRM/thetadatadx/sdks/go"

creds, err := thetadatadx.CredentialsFromFile("creds.txt")
if err != nil { log.Fatal(err) }
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil { log.Fatal(err) }
defer client.Close()
```

Env-var variant:

```go
creds, err := thetadatadx.CredentialsFromEnv("THETA_EMAIL", "THETA_PASS")
if err != nil { log.Fatal(err) }
defer creds.Close()
```

## Historical call

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil { log.Fatal(err) }
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil { log.Fatal(err) }
    defer client.Close()

    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil { log.Fatal(err) }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
    }
}
```

Prices are decoded to `float64` at parse time — no `Price{value, type}` wrappers.

## Streaming call

The Go streaming client is a separate type (`FpssClient`), independent of the historical client. This keeps handle lifetimes and memory ownership unambiguous across the CGo boundary.

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil { log.Fatal(err) }
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil { log.Fatal(err) }
    defer fpss.Close()

    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeTrades("MSFT")

    for {
        event, err := fpss.NextEvent(1000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil { continue }

        switch event.Kind {
        case thetadatadx.FpssQuoteEvent:
            q := event.Quote
            fmt.Printf("Quote: %d %.2f/%.2f\n", q.ContractID, q.Bid, q.Ask)
        case thetadatadx.FpssTradeEvent:
            t := event.Trade
            fmt.Printf("Trade: %d %.2f x %d\n", t.ContractID, t.Price, t.Size)
        }
    }

    fpss.Shutdown()
}
```

## Error handling

```go
ticks, err := client.OptionHistoryGreeksAll("SPY", "20240419", "500", "C",
    "20240101", "20240301")

var rateErr *thetadatadx.RateLimitError
var subErr *thetadatadx.SubscriptionError
var authErr *thetadatadx.AuthError

switch {
case errors.As(err, &rateErr):
    time.Sleep(time.Duration(rateErr.WaitMs) * time.Millisecond)
    // retry
case errors.As(err, &subErr):
    log.Printf("%s requires %s", subErr.Endpoint, subErr.RequiredTier)
case errors.As(err, &authErr):
    refreshCredentials()
case err != nil:
    return err
}
```

## Next

- [Historical data](../historical/) — 61 endpoints
- [Streaming (FPSS)](../streaming/) — polling model, event types, reconnect
- [Options & Greeks](../options) — wildcard chain queries, local Greeks calculator
- [Error handling](../getting-started/errors) — full `ThetaDataError` hierarchy
