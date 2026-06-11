---
title: Error Codes
description: The typed error surface across all four SDKs and the server's error envelope.
---

# Error Codes

One error model spans the SDK: the Rust core classifies every failure once, and each language surfaces the classification in its idiom. Catch the specific class you can recover from; let the rest propagate.

[Download as CSV](/csv/error-types.csv)

| Condition | Rust `Error` variant | Python exception | C++ exception |
|---|---|---|---|
| Bad or expired credentials | `Auth` | `AuthenticationError` / `InvalidCredentialsError` | `tdx::AuthenticationError` / `tdx::InvalidCredentialsError` |
| Endpoint needs a higher tier | `Grpc` (permission) | `SubscriptionError` | `tdx::SubscriptionError` |
| Too many requests in flight upstream | `Grpc` (resource exhausted) | `RateLimitError` | `tdx::RateLimitError` |
| Request returned no rows | `NoData` | `NoDataFoundError` | `tdx::Error` with `kind == NoData` |
| Per-request deadline elapsed | `Timeout` | `TimeoutError` | `tdx::Error` with `kind == Timeout` |
| Connection / TLS / protocol fault | `Transport` | `NetworkError` | `tdx::NetworkError` |
| Response shape unexpected | `Decode` | `SchemaMismatchError` | `tdx::SchemaMismatchError` |
| Streaming session fault | `Fpss` | `StreamError` | `tdx::StreamError` |
| Invalid parameters / configuration | `Config` | `ThetaDataError` | `tdx::ThetaDataError` |

- **Python** exceptions all derive from `ThetaDataError`, so `except ThetaDataError` is the catch-all.
- **TypeScript** throws the standard `Error`; the message carries the same stable text as the Rust `Display` output, so the failure category is recognizable without a class tree.
- **C++** exceptions derive from `tdx::ThetaDataError`; `NoData` and `Timeout` ride the generic `tdx::Error` with a `kind` discriminator.

```python
from thetadatadx import NoDataFoundError, RateLimitError, ThetaDataError

try:
    rows = tdx.option_history_trade("SPY", "20250321", "20250303", strike="570", right="C")
except NoDataFoundError:
    rows = []                # nothing traded — a normal outcome, not a failure
except RateLimitError:
    ...                      # back off and retry; see Concurrent Requests
except ThetaDataError:
    raise                    # anything else is a real failure
```

Transient faults (transport drops, upstream exhaustion) are retried inside the SDK with backoff before any error surfaces; tune the budget via the `retry_*` [configuration](/articles/configuration) fields.

## Server error envelope

The [HTTP server](/server/http) reports every failure with one envelope shape and an `error_type` discriminator:

```json
{
    "header": { "error_type": "bad_request", "error_msg": "missing required parameter: 'date' (Date YYYYMMDD)" },
    "response": []
}
```

| HTTP status | `error_type` | Meaning |
|---|---|---|
| 400 | `bad_request` | Missing or invalid parameter; the message names it. |
| 404 | `not_found` | Unknown route. |
| 429 | — | Per-IP rate limit on non-loopback binds; carries `Retry-After`. |
| 503 | `upstream_exhausted` | Upstream capacity exhausted after retries; carries `Retry-After`. |
