# Real-Time Streaming (Python)

Real-time market data via ThetaData's FPSS servers. The Python SDK uses a polling model with `next_event()`.

## Connect

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
```

## Subscribe

```python
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")
tdx.subscribe_open_interest("SPY")
```

## Receive Events

Events are returned as Python dicts with a `"type"` field. `next_event()` returns `None` on timeout.

```python
# Track contract_id -> symbol mapping
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
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

## Stop Streaming

```python
tdx.stop_streaming()
```

## Streaming Methods (on ThetaDataDx)

| Method | Description |
|--------|-------------|
| `start_streaming()` | Connect to FPSS streaming servers |
| `subscribe_quotes(symbol)` | Subscribe to quote data |
| `subscribe_trades(symbol)` | Subscribe to trade data |
| `subscribe_open_interest(symbol)` | Subscribe to open interest |
| `next_event(timeout_ms=5000)` | Poll next event (dict or `None`) |
| `stop_streaming()` | Graceful shutdown of streaming |

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
from thetadatadx import Credentials, Config, ThetaDataDx
import signal
import sys

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Start streaming
tdx.start_streaming()

# Graceful shutdown on Ctrl+C
def shutdown_handler(sig, frame):
    tdx.stop_streaming()
    sys.exit(0)

signal.signal(signal.SIGINT, shutdown_handler)

# Subscribe to multiple streams
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("AAPL")
tdx.subscribe_quotes("MSFT")

contracts = {}

while True:
    event = tdx.next_event(timeout_ms=5000)
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

tdx.stop_streaming()
```
