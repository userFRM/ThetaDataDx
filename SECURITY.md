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

Security fixes land on the current major line only. Older majors are not patched; upgrade to the latest `12.x` release.

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 12.x    | :white_check_mark: | Current release |
| 9.x-11.x | :x:               | Upgrade to 12.x |
| < 9.0   | :x:                | Upgrade to 12.x |

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
- **Session UUIDs** (bearer tokens for MDDS requests) are logged at `debug!` level only,
  redacted to first 8 characters. They never appear at `info!` or higher.
- Credentials are not persisted to disk by the library (the `creds.txt` file is
  user-managed and excluded from version control via `.gitignore`)

### Timeouts

All network operations enforce timeouts to prevent indefinite hangs:

- **Nexus auth HTTP**: 10s request timeout, 5s connect timeout
- **MDDS**: connect timeout + keepalive from `DirectConfig`
- **FPSS TLS**: connect timeout wraps both TCP and TLS handshake
- **FPSS read loop**: read timeout matching Java's `SO_TIMEOUT=10s`

### TLS

All network connections use a **unified TLS stack** (`rustls` with ring backend):

- **MDDS**: TLS via `rustls`
- **FPSS (streaming)**: TLS via `tokio-rustls` + `rustls`
- **Nexus auth (HTTP)**: TLS via `reqwest` + `rustls`

Root certificates come from `webpki-roots` (Mozilla's CA bundle). Certificate
validation is enforced on MDDS (gRPC) and Nexus (HTTP) connections. FPSS (streaming)
skips certificate verification because ThetaData's FPSS servers have certificates
expired since January 2024 -- this matches the Java terminal's behavior.

### Credential Handling (FPSS)

FPSS credential length fields are read as unsigned integers (matching Java's
`readUnsignedShort()`), so passwords longer than 127 bytes authenticate correctly
(a signed read would sign-extend the length and break the handshake).

### Concurrent Request Limiting

The SDK caps the number of in-flight historical requests with an internal semaphore. The cap
is derived automatically from the account's subscription tier at connect time and is not
user-configurable. This respects the server-side per-tier concurrency limit and prevents
runaway request storms from overwhelming the upstream server or triggering server-side rate
limiting.

### Unknown Compression Rejection

`decompress_response` returns an error for unrecognized compression algorithms
instead of silently treating the data as uncompressed. This prevents corrupt data from being
silently passed to callers.

### FPSS Event Dispatch

FPSS streaming uses a fully synchronous I/O thread with a lock-free event ring buffer
for event dispatch. The bounded ring buffer prevents unbounded memory
growth from unconsumed events.

### Frame Size Limits

Binary frame size assertions use `assert!` (not `debug_assert!`), ensuring they
are enforced in release builds. This prevents oversized frames from causing
unbounded memory allocation.

### Dependencies

We review dependencies for:
- Known vulnerabilities (RustSec advisory database)
- License compliance
- Duplicate crate versions
