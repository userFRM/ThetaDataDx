# Real-Time Streaming (Go)

Real-time market data via ThetaData's FPSS servers. The Go SDK uses a polling model with `NextEvent()`.

## Connect

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()

client.StartStreaming(1024)
```

## Subscribe

```go
// Stock quotes
reqID, _ := client.SubscribeQuotes("AAPL", thetadatadx.SecTypeStock)
fmt.Printf("Subscribed (req_id=%d)\n", reqID)

// Stock trades
client.SubscribeTrades("MSFT", thetadatadx.SecTypeStock)

// Open interest
client.SubscribeOpenInterest("AAPL", thetadatadx.SecTypeStock)
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
    event, err := client.NextEvent(5000) // 5s timeout
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

## Stop Streaming

```go
client.StopStreaming()
```

## Streaming Methods (on Client)

| Method | Signature | Description |
|--------|-----------|-------------|
| `StartStreaming` | `(bufSize) error` | Connect to FPSS streaming servers |
| `SubscribeQuotes` | `(root, secType) (int32, error)` | Subscribe to quotes |
| `SubscribeTrades` | `(root, secType) (int32, error)` | Subscribe to trades |
| `SubscribeOpenInterest` | `(root, secType) (int32, error)` | Subscribe to OI |
| `NextEvent` | `(timeoutMs) (*FpssEvent, error)` | Poll next event |
| `StopStreaming` | `() error` | Graceful shutdown of streaming |

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

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Start streaming and subscribe
    client.StartStreaming(1024)
    client.SubscribeQuotes("AAPL", thetadatadx.SecTypeStock)
    client.SubscribeTrades("AAPL", thetadatadx.SecTypeStock)

    // Process events
    for {
        event, err := client.NextEvent(5000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue
        }
        fmt.Printf("Event: %+v\n", event)
    }

    client.StopStreaming()
}
```
