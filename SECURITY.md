# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in thetadatadx, please report it responsibly:

1. **Do NOT open a public GitHub issue** for security vulnerabilities
2. Open a **private security advisory** on GitHub: Repository > Security > Advisories > New
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

We will acknowledge receipt within 48 hours and aim to release a fix within 7 days
for critical issues.

## Supported Versions

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 5.2.x   | :white_check_mark: | Current release (f64 prices, `#[repr(C)]` FPSS events, contract ID fields) |
| 5.0-5.1 | :x:                | Upgrade to 5.2.x (Go price formula bug fixed in 5.2) |
| 4.5.x   | :x:                | Pre-builder-pattern API, missing f64 convenience methods |
| 4.0-4.4 | :x:                | Timezone bug (ms_of_day shifted +1 hour Nov-Mar) |
| 3.x     | :x:                | Pre-`tdbe` extraction, stale API |
| < 3.0   | :x:                | Contract wire format bug, missing endpoints |

> **Important:** Versions prior to 4.5.0 contain a timezone bug that shifts ms_of_day by +1 hour
> for all historical data from November through March (EST period). Versions prior to 5.2.0
> contain a Go SDK price formula bug where `PriceToF64` used the wrong formula. All users
> should upgrade to 5.2.x.

## Security Design

### Terminal API Key

The ThetaData terminal ships with a hardcoded API key that is identical across all
installations. **This is not a secret** — it is a protocol constant embedded in every
copy of the Java terminal. thetadatadx includes this key for protocol compatibility. It
provides no privileged access.

### Credential Handling

- User credentials (email/password) are used for both Nexus auth and FPSS authentication
- The `Debug` trait implementation for `Credentials` **redacts** passwords — they
  never appear in debug output or log lines
- `AuthRequest` (internal HTTP body struct) does **not** derive `Debug` — prevents
  accidental password exposure in error traces
- **Session UUIDs** (bearer tokens for MDDS gRPC) are logged at `debug!` level only,
  redacted to first 8 characters. They never appear at `info!` or higher.
- Credentials are not persisted to disk by the library (the `creds.txt` file is
  user-managed and excluded from version control via `.gitignore`)

### Timeouts

All network operations enforce timeouts to prevent indefinite hangs:

- **Nexus auth HTTP**: 10s request timeout, 5s connect timeout
- **MDDS gRPC**: connect timeout + keepalive from `DirectConfig`
- **FPSS TCP+TLS**: connect timeout wraps both TCP and TLS handshake
- **FPSS read loop**: read timeout matching Java's `SO_TIMEOUT=10s`

### TLS

All network connections use a **unified TLS stack** (`rustls` with ring backend):

- **MDDS (gRPC)**: TLS via `tonic` + `rustls`
- **FPSS (streaming)**: TLS via `tokio-rustls` + `rustls`
- **Nexus auth (HTTP)**: TLS via `reqwest` + `rustls`

Root certificates come from `webpki-roots` (Mozilla's CA bundle). Certificate
validation is enforced on MDDS (gRPC) and Nexus (HTTP) connections. FPSS (streaming)
skips certificate verification because ThetaData's FPSS servers have certificates
expired since January 2024 -- this matches the Java terminal's behavior.

### Credential Handling (FPSS)

As of v1.2.0, FPSS credential length fields are read as unsigned integers (matching Java's
`readUnsignedShort()`). Previous versions used signed reads, which could cause a sign-extension
bug for passwords longer than 127 bytes. This did not leak credentials but could cause
authentication failures.

### Concurrent Request Limiting

DirectClient enforces a semaphore (`mdds_concurrent_requests`) that limits the number of
in-flight gRPC requests. The default is dynamically derived from the user's subscription
tier (`2^tier`), matching the Java terminal's concurrency model. This prevents runaway
request storms from overwhelming the upstream MDDS server or triggering server-side rate
limiting.

### Unknown Compression Rejection

As of v1.2.0, `decompress_response` returns an error for unrecognized compression algorithms
instead of silently treating the data as uncompressed. This prevents corrupt data from being
silently passed to callers.

### FPSS Event Dispatch

FPSS streaming uses a fully synchronous I/O thread with a lock-free disruptor ring buffer
(`disruptor-rs` v4) for event dispatch. The bounded ring buffer prevents unbounded memory
growth from unconsumed events.

### Frame Size Limits

Binary frame size assertions use `assert!` (not `debug_assert!`), ensuring they
are enforced in release builds. This prevents oversized frames from causing
unbounded memory allocation.

### Dependencies

We audit dependencies for:
- Known vulnerabilities (RustSec advisory database)
- License compliance
- Duplicate crate versions
