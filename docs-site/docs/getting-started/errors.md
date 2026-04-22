---
title: Error Handling
description: ThetaDataError hierarchy, retry policy, session auto-refresh, and timeouts in ThetaDataDx.
---

# Error Handling

Errors surface through a layered `ThetaDataError` hierarchy so callers can narrow an `except` clause to the specific failure they want to recover from. The Rust core defines the enum once; each SDK exposes it in its language idiom.

## Exception hierarchy

```
ThetaDataError (base)
├── AuthError              -- bad credentials, expired session
├── SubscriptionError      -- endpoint requires a higher tier
├── RateLimitError         -- TooManyRequests (code 12)
├── EndpointNotFoundError  -- server returned NotFound for a known endpoint
├── SchemaMismatchError    -- decoder cannot unpack the response
├── NetworkError           -- connect / TLS / stream failures
└── TimeoutError           -- deadline exceeded
```

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Error};

match tdx.option_history_greeks_all("SPY", "20240419", "500", "C", "20240101", "20240301").await {
    Ok(ticks) => process(ticks),
    Err(Error::RateLimited { wait_ms, .. }) => {
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        // retry
    }
    Err(Error::Subscription { endpoint, required_tier }) => {
        eprintln!("{endpoint} requires {required_tier}");
    }
    Err(Error::Auth(_)) => refresh_credentials(),
    Err(err) => return Err(err),
}
```
```python [Python]
from thetadatadx import ThetaDataError, AuthError, SubscriptionError, RateLimitError

try:
    ticks = tdx.option_history_greeks_all("SPY", "20240419", "500", "C",
                                          "20240101", "20240301")
except RateLimitError as e:
    time.sleep(e.wait_seconds)
    # retry
except SubscriptionError as e:
    print(f"Endpoint {e.endpoint} requires {e.required_tier}")
except AuthError:
    refresh_credentials()
except ThetaDataError as e:
    logger.exception("unexpected SDK failure")
    raise
```
```typescript [TypeScript]
import {
    ThetaDataError, AuthError, SubscriptionError, RateLimitError,
} from 'thetadatadx';

try {
    const ticks = tdx.optionHistoryGreeksAll('SPY', '20240419', '500', 'C',
                                             '20240101', '20240301');
} catch (e) {
    if (e instanceof RateLimitError) {
        await new Promise(r => setTimeout(r, e.waitMs));
        // retry
    } else if (e instanceof AuthError) {
        refreshCredentials();
    } else {
        throw e;
    }
}
```
```go [Go]
ticks, err := client.OptionHistoryGreeksAll("SPY", "20240419", "500", "C",
    "20240101", "20240301")

var rateErr *thetadatadx.RateLimitError
var authErr *thetadatadx.AuthError
switch {
case errors.As(err, &rateErr):
    time.Sleep(time.Duration(rateErr.WaitMs) * time.Millisecond)
    // retry
case errors.As(err, &authErr):
    refreshCredentials()
case err != nil:
    return err
}
```
```cpp [C++]
try {
    auto ticks = client.option_history_greeks_all("SPY", "20240419", "500", "C",
                                                  "20240101", "20240301");
} catch (const tdx::RateLimitError& e) {
    std::this_thread::sleep_for(std::chrono::milliseconds(e.wait_ms()));
    // retry
} catch (const tdx::AuthError&) {
    refresh_credentials();
} catch (const tdx::ThetaDataError& e) {
    std::cerr << e.what() << std::endl;
    throw;
}
```
:::

## Retry policy

The SDK does not retry historical calls automatically. Callers decide — network blips, rate limits, and timeouts are explicit signals you may want to route through different code paths. The concrete knobs:

| Failure | Auto-handled | Caller action |
|---------|--------------|---------------|
| `AuthError` — session expired | No | `refresh_credentials()` and retry |
| `AuthError` — bad credentials | No | Surface to user; retrying will not help |
| `RateLimitError` (TooManyRequests) | No | Sleep `wait_ms` (server-provided), then retry |
| `NetworkError` — connection reset | No | Short backoff then retry |
| `TimeoutError` | No | Increase timeout or retry |
| `SubscriptionError` | No | Wrong tier; retrying will not help |
| `SchemaMismatchError` | No | File a bug (decoder drift) |

A recommended shape — tenacity for Python, `tokio-retry` for Rust, a handwritten loop for Go/C++/TypeScript — is:

```python
from tenacity import retry, retry_if_exception_type, wait_exponential, stop_after_attempt
from thetadatadx import RateLimitError, NetworkError, TimeoutError

@retry(
    retry=retry_if_exception_type((RateLimitError, NetworkError, TimeoutError)),
    wait=wait_exponential(multiplier=1, min=2, max=60),
    stop=stop_after_attempt(5),
)
def pull_chain(tdx, symbol, exp):
    return tdx.option_snapshot_quote(symbol, exp, "*", "both")
```

## Session auto-refresh

The client owns a session UUID returned by ThetaData's Nexus auth endpoint. It attaches the UUID to every gRPC call and every FPSS frame. On an `Unauthenticated` gRPC status, the client re-runs the auth exchange once automatically and retries the call with the fresh UUID before surfacing `AuthError` to the caller. This covers the common case of a session expiring mid-pipeline without forcing the caller to wrap every endpoint call.

Re-auth is one-shot: if the fresh UUID also fails, the underlying cause is a credential problem, not a session lifecycle one, and the error propagates.

## Timeouts

Every timeout is configurable through `DirectConfig` / `Config`. Defaults aimed at a well-behaved production deployment:

| Timeout | Default | Configurable as |
|---------|--------:|-----------------|
| Nexus HTTP connect | 5,000 ms | `nexus_connect_timeout_ms` |
| Nexus HTTP request | 10,000 ms | `nexus_request_timeout_ms` |
| MDDS gRPC keepalive | 30,000 ms | `mdds_keepalive_secs` |
| FPSS connect | 2,000 ms | `fpss_connect_timeout_ms` |
| FPSS read | 10,000 ms | `fpss_timeout_ms` |
| FPSS ping | 100 ms | `fpss_ping_interval_ms` |
| Reconnect (normal) | 2,000 ms | `reconnect_wait_ms` |
| Reconnect (rate-limited) | 130,000 ms | `reconnect_wait_rate_limited_ms` |

See [Configuration](../configuration) for the full struct.

## Next

- [Quick Start](./quickstart) — install + first call + first stream, tabbed per language
- [DataFrames](./dataframes) — Arrow / Polars / Pandas output with the zero-copy scope
