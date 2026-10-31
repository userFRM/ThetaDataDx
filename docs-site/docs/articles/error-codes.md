---
title: Error Codes
description: The typed error surface across all four SDKs and the server's error responses.
---

# Error Codes

One error model spans the SDK: the Rust core classifies every failure once, and each language surfaces the classification in its idiom. Catch the specific class you can recover from; let the rest propagate.

[Download as CSV](/csv/error-types.csv)

| Condition | Rust `Error` variant | Python exception | C++ exception |
|---|---|---|---|
| Bad or expired credentials | `Auth` | `AuthenticationError` / `InvalidCredentialsError` | `thetadatadx::AuthenticationError` / `thetadatadx::InvalidCredentialsError` |
| Endpoint needs a higher tier | `Grpc` (permission) | `SubscriptionError` | `thetadatadx::SubscriptionError` |
| Too many requests in flight upstream | `Grpc` (resource exhausted) | `RateLimitError` | `thetadatadx::RateLimitError` |
| Upstream service unavailable | `Grpc` (unavailable) | `UnavailableError` | `thetadatadx::UnavailableError` |
| Request returned no rows | `Grpc` (not found) | `NotFoundError` / `NoDataFoundError` | `thetadatadx::NotFoundError` |
| Per-request deadline elapsed | `Timeout` | `DeadlineExceededError` / `TimeoutError` | `thetadatadx::DeadlineExceededError` |
| Connection / TLS / protocol fault | `Transport` | `NetworkError` | `thetadatadx::NetworkError` |
| Response shape unexpected | `Decode` | `SchemaMismatchError` | `thetadatadx::SchemaMismatchError` |
| Streaming session fault | `Stream` | `StreamError` | `thetadatadx::StreamError` |
| Invalid parameter or configuration value | `Config` (invalid value) | `InvalidParameterError` | `thetadatadx::InvalidParameterError` |
| Configuration file or environment fault | `Config` (other) | `ConfigError` | `thetadatadx::ConfigError` |

- **Python** exceptions all derive from `ThetaDataError`, so `except ThetaDataError` is the catch-all.
- **TypeScript** throws a typed subclass, the same hierarchy the other bindings expose: `catch (e) { if (e instanceof SubscriptionError) ... }`. A rate-limit error carries `retryAfter`. The message still carries the same stable text as the Rust `Display` output, but an instance check is the reliable test, not a pattern over the message.
- **C++** exceptions derive from `thetadatadx::ThetaDataError`, so `catch (const thetadatadx::ThetaDataError&)` is the catch-all.

```python
from thetadatadx import NoDataFoundError, RateLimitError, ThetaDataError

try:
    rows = client.market_data.option_history_trade("SPY", "20250321", "20250303", strike="570", right="C")
except NoDataFoundError:
    rows = []                # nothing traded: a normal outcome, not a failure
except RateLimitError:
    ...                      # back off and retry; see Concurrent Requests
except ThetaDataError:
    raise                    # anything else is a real failure
```

Transient faults (transport drops, upstream exhaustion) are retried inside the SDK with backoff before any error surfaces; tune the budget via the `retry_*` [configuration](/articles/configuration) fields.

## Server errors

The [HTTP server](/server/http) answers every failed request the way the terminal does: with an HTTP status and the reason as a plain-text (`text/plain`) body.

| HTTP status | Meaning |
|---|---|
| 400 | Missing or invalid parameter, or more than 32 query parameters; the body names it. |
| 404 | Unknown endpoint; the body names it. A path the server does not serve at all gets an empty 404. |
| 410 | A v2 query parameter; the body names its v3 replacement. |
| 503 | Upstream capacity exhausted after retries; carries `Retry-After`. |
| Set by the upstream | The upstream service rejected the request. The server answers with the HTTP status the service attaches to the rejection (for example its no-data status when a query matches no rows) and the service's description, or with 500 when the service attaches no status. |
