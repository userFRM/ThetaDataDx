# Real-Time Streaming (Go)

Real-time market data via ThetaData's FPSS servers. The Go SDK uses a polling model with `NextEvent()`.

## Connect

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

fpss, err := thetadatadx.FpssConnect(creds, 1024)
if err != nil {
    log.Fatal(err)
}
defer fpss.Shutdown()
```

## Subscribe

```go
// Stock quotes
reqID, _ := fpss.SubscribeQuotes("AAPL", thetadatadx.SecTypeStock)
fmt.Printf("Subscribed (req_id=%d)\n", reqID)

// Stock trades
fpss.SubscribeTrades("MSFT", thetadatadx.SecTypeStock)

// Open interest
fpss.SubscribeOpenInterest("AAPL", thetadatadx.SecTypeStock)
```

### Security Type Constants

```go
thetadatadx.SecTypeStock   // 0
thetadatadx.SecTypeOption  // 1
thetadatadx.SecTypeIndex   // 2
thetadatadx.SecTypeRate    // 3
```

## Receive Events

`NextEvent()` returns `nil` on timeout.

```go
for {
    event, err := fpss.NextEvent(5000) // 5s timeout
    if err != nil {
        log.Println("Error:", err)
        break
    }
    if event == nil {
        continue // timeout
    }
    fmt.Printf("Event: %+v\n", event)
}
```

## Shutdown

```go
fpss.Shutdown()
```

## FpssClient Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `FpssConnect` | `(creds, bufSize) (*FpssClient, error)` | Connect and authenticate |
| `SubscribeQuotes` | `(root, secType) (int32, error)` | Subscribe to quotes |
| `SubscribeTrades` | `(root, secType) (int32, error)` | Subscribe to trades |
| `SubscribeOpenInterest` | `(root, secType) (int32, error)` | Subscribe to OI |
| `NextEvent` | `(timeoutMs) (*FpssEvent, error)` | Poll next event |
| `Shutdown` | `() error` | Graceful shutdown |

## Complete Example

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

    fpss, err := thetadatadx.FpssConnect(creds, 1024)
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Shutdown()

    // Subscribe to quotes and trades
    fpss.SubscribeQuotes("AAPL", thetadatadx.SecTypeStock)
    fpss.SubscribeTrades("AAPL", thetadatadx.SecTypeStock)

    // Process events
    for {
        event, err := fpss.NextEvent(5000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue
        }
        fmt.Printf("Event: %+v\n", event)
    }
}
```
