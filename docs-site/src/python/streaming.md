# Real-Time Streaming (Python)

Real-time market data via ThetaData's FPSS servers. The Python SDK uses a polling model with `next_event()`.

## Connect

```python
from thetadatadx import Credentials, FpssClient

creds = Credentials.from_file("creds.txt")
fpss = FpssClient(creds, buffer_size=1024)
```

## Subscribe

```python
fpss.subscribe("AAPL", "QUOTE")
fpss.subscribe("MSFT", "TRADE")
fpss.subscribe("SPY", "OI")
```

## Receive Events

Events are returned as Python dicts with a `"type"` field. `next_event()` returns `None` on timeout.

```python
# Track contract_id -> symbol mapping
contracts = {}

while True:
    event = fpss.next_event(timeout_ms=5000)
    if event is None:
        continue  # timeout, no event

    # Control events
    if event["type"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
        print(f"Contract {event['id']} = {event['contract']}")
        continue

    if event["type"] == "login_success":
        print(f"Logged in: {event['permissions']}")
        continue

    # Data events
    if event["type"] == "quote":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Quote: {symbol} bid={event['bid']} ask={event['ask']}")

    elif event["type"] == "trade":
        contract_id = event["contract_id"]
        symbol = contracts.get(contract_id, f"id={contract_id}")
        print(f"Trade: {symbol} price={event['price']} size={event['size']}")

    elif event["type"] == "open_interest":
        print(f"OI: contract={event['contract_id']} oi={event['open_interest']}")

    elif event["type"] == "ohlcvc":
        print(f"OHLCVC: contract={event['contract_id']} "
              f"O={event['open']} H={event['high']} L={event['low']} C={event['close']}")

    elif event["type"] == "disconnected":
        print(f"Disconnected: {event['reason']}")
        break
```

## Shutdown

```python
fpss.shutdown()
```

## FpssClient Methods

| Method | Description |
|--------|-------------|
| `FpssClient(creds, buffer_size=1024)` | Connect and authenticate |
| `subscribe(symbol, data_type)` | Subscribe (`"QUOTE"`, `"TRADE"`, `"OI"`) |
| `next_event(timeout_ms=5000)` | Poll next event (dict or `None`) |
| `shutdown()` | Graceful shutdown |

## Event Types

### Data Events

| `type` | Key Fields |
|--------|------------|
| `"quote"` | `contract_id`, `bid`, `ask`, `bid_size`, `ask_size`, `price_type`, `date` |
| `"trade"` | `contract_id`, `price`, `size`, `exchange`, `condition`, `price_type`, `date` |
| `"open_interest"` | `contract_id`, `open_interest`, `date` |
| `"ohlcvc"` | `contract_id`, `open`, `high`, `low`, `close`, `volume`, `count`, `date` |

### Control Events

| `type` | Key Fields |
|--------|------------|
| `"login_success"` | `permissions` |
| `"contract_assigned"` | `id`, `contract` |
| `"req_response"` | `req_id`, `result` |
| `"market_open"` | (none) |
| `"market_close"` | (none) |
| `"disconnected"` | `reason` |
| `"error"` | `message` |

## Complete Example

```python
from thetadatadx import Credentials, FpssClient
import signal
import sys

creds = Credentials.from_file("creds.txt")
fpss = FpssClient(creds, buffer_size=1024)

# Graceful shutdown on Ctrl+C
def shutdown_handler(sig, frame):
    fpss.shutdown()
    sys.exit(0)

signal.signal(signal.SIGINT, shutdown_handler)

# Subscribe to multiple streams
fpss.subscribe("AAPL", "QUOTE")
fpss.subscribe("AAPL", "TRADE")
fpss.subscribe("MSFT", "QUOTE")

contracts = {}

while True:
    event = fpss.next_event(timeout_ms=5000)
    if event is None:
        continue

    if event["type"] == "contract_assigned":
        contracts[event["id"]] = event["contract"]
    elif event["type"] == "quote":
        name = contracts.get(event["contract_id"], "?")
        print(f"[QUOTE] {name}: bid={event['bid']} ask={event['ask']}")
    elif event["type"] == "trade":
        name = contracts.get(event["contract_id"], "?")
        print(f"[TRADE] {name}: price={event['price']} size={event['size']}")
    elif event["type"] == "disconnected":
        print(f"Disconnected: {event['reason']}")
        break

fpss.shutdown()
```
