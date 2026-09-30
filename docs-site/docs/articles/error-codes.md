---
title: Error Codes
description: The typed error surface across all four SDKs and the server's error envelope.
---

# Error Codes

One error model spans the SDK: the Rust core classifies every failure once, and each language surfaces the classification in its idiom. Catch the specific class you can recover from; let the rest propagate.

[Download as CSV](/csv/error-types.csv)

| Condition | Rust `Error` variant | Python exception | C++ exception |
|---|---|---|---|
| Bad or expired credentials | `Auth` | `AuthenticationError` / `InvalidCredentialsError` | `thetadatadx::AuthenticationError` / `thetadatadx::InvalidCredentialsError` |
| Endpoint needs a higher tier | `Grpc` (permission) | `SubscriptionError` | `thetadatadx::SubscriptionError` |
| Too many requests in flight upstream | `Grpc` (resource exhausted) | `RateLimitError` | `thetadatadx::RateLimitError` |
| Request returned no rows | `Grpc` (not found) | `NotFoundError` / `NoDataFoundError` | `thetadatadx::NotFoundError` |
| Per-request deadline elapsed | `Timeout` | `DeadlineExceededError` / `TimeoutError` | `thetadatadx::DeadlineExceededError` |
| Connection / TLS / protocol fault | `Transport` | `NetworkError` | `thetadatadx::NetworkError` |
| Response shape unexpected | `Decode` | `SchemaMismatchError` | `thetadatadx::SchemaMismatchError` |
| Streaming session fault | `Stream` | `StreamError` | `thetadatadx::StreamError` |
| Invalid parameters / configuration | `Config` | `ThetaDataError` | `thetadatadx::ThetaDataError` |

- **Python** exceptions all derive from `ThetaDataError`, so `except ThetaDataError` is the catch-all.
- **TypeScript** throws a typed subclass, the same hierarchy the other bindings expose: `catch (e) { if (e instanceof SubscriptionError) ... }`. A rate-limit error carries `retryAfter`. The message still carries the same stable text as the Rust `Display` output, but an instance check is the reliable test, not a pattern over the message.
- **C++** exceptions derive from `thetadatadx::ThetaDataError`, so `catch (const thetadatadx::ThetaDataError&)` is the catch-all.

```python
from thetadatadx import NoDataFoundError, RateLimitError, ThetaDataError

try:
    rows = client.market_data.option_history_trade("SPY", "20250321", "20250303", strike="570", right="C")
except NoDataFoundError:
    rows = []                # nothing traded — a normal outcome, not a failure
except RateLimitError:
    ...                      # back off and retry; see Concurrent Requests
except ThetaDataError:
    raise                    # anything else is a real failure
```

Transient faults (transport drops, upstream exhaustion) are retried inside the SDK with backoff before any error surfaces; tune the budget via the `retry_*` [configuration](/articles/configuration) fields.

## Server errors

The [HTTP server](/server/http) answers a failed data request the way the terminal does: with an HTTP status and the reason as a plain-text (`text/plain`) body.

| HTTP status | Meaning |
|---|---|
| 400 | Missing or invalid parameter; the body names it. |
| 404 | Unknown route. |
| 410 | A v2 query parameter; the body names its v3 replacement. |
| 503 | Upstream capacity exhausted after retries; carries `Retry-After`. |
| Set by the upstream | The upstream service rejected the request. The server answers with the HTTP status the service attaches to the rejection (for example its no-data status when a query matches no rows) and the service's description, or with 500 when the service attaches no status. |

Two kinds of failure are answered with a JSON envelope instead, whose `error_type` names the class: a query string the server cannot accept (more than 32 parameters, or one it cannot parse), and every failure on the [flat-file](/articles/flat-files) routes.

```json
{
    "header": { "error_type": "bad_request", "error_msg": "request has 40 query parameters; max is 32" },
    "response": []
}
```

| HTTP status | `error_type` | Meaning |
|---|---|---|
| 400 | `bad_request` | The query string or a flat-file parameter was rejected; the message names it. |
| 404 | `flatfiles_no_data` | No flat file is available to this account for the requested date. |
| 502 | `flatfiles_unavailable` | The upstream failed while serving the flat file. |
