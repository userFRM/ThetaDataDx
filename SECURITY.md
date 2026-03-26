# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in thetadatadx, please report it responsibly:

1. **Do NOT open a public GitHub issue** for security vulnerabilities
2. Email: **security@thetadatadx.dev** (or open a private security advisory on GitHub)
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

We will acknowledge receipt within 48 hours and aim to release a fix within 7 days
for critical issues.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

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
validation is enforced on all connections — there is no option to skip verification.

### Concurrent Request Limiting

DirectClient enforces a configurable semaphore (`mdds_concurrent_requests`, default 2) that
limits the number of in-flight gRPC requests. This prevents runaway request storms from
overwhelming the upstream MDDS server or triggering server-side rate limiting. The default
matches the most common ThetaData tier.

### FPSS Event Dispatch

FPSS streaming uses a fully synchronous I/O thread with a lock-free disruptor ring buffer
(`disruptor-rs` v4) for event dispatch. The bounded ring buffer prevents unbounded memory
growth from unconsumed events.

### Frame Size Limits

Binary frame size assertions use `assert!` (not `debug_assert!`), ensuring they
are enforced in release builds. This prevents oversized frames from causing
unbounded memory allocation.

### Dependencies

We use `cargo-deny` to audit dependencies for:
- Known vulnerabilities (RustSec advisory database)
- License compliance
- Duplicate crate versions

See `deny.toml` for the full configuration.
