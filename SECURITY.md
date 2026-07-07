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

Security fixes land on the latest release. This is the project's first public release line, so there are no earlier supported versions.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Security Design

### Terminal API Key

The ThetaData terminal ships with a hardcoded API key that is identical across all
installations. **This is not a secret** — it is a protocol constant embedded in every
official terminal copy. thetadatadx includes this key for protocol compatibility. It
provides no privileged access.

### Credential Handling

- User credentials (email/password) are used for both account authentication and streaming authentication
- The `Debug` trait implementation for `Credentials` **redacts** passwords — they
  never appear in debug output or log lines
- Authentication request bodies do **not** derive `Debug` — prevents accidental password
  exposure in error traces
- **Session UUIDs** (bearer tokens for market-data requests) are logged at `debug!` level only,
  redacted to first 8 characters. They never appear at `info!` or higher.
- Credentials are not persisted to disk by the library (the `creds.txt` file is
  user-managed and excluded from version control via `.gitignore`)

### Timeouts

All network operations enforce timeouts to prevent indefinite hangs:

- **Nexus auth HTTP**: 10s request timeout, 5s connect timeout
- **Market-data channel**: connect timeout + keepalive from `DirectConfig`
- **Streaming TLS**: connect timeout wraps both TCP and TLS handshake
- **Streaming reads**: a 10s read timeout matching the official terminal behavior

### TLS

All network connections use a single, unified TLS stack:

- **Market-data channel**: TLS with certificate validation
- **Streaming channel**: TLS with certificate validation and pinning
- **Nexus auth (HTTP)**: TLS with certificate validation

Root certificates come from Mozilla's root CA bundle. Certificate
validation is enforced on market-data and Nexus (HTTP) connections. Streaming
uses a pinned verifier: it accepts only the configured streaming hostnames and the expected
leaf `SubjectPublicKeyInfo` SHA-256 pin, while still verifying the TLS handshake
signature.

### Credential Handling (Streaming)

Streaming credential length fields are read as an unsigned 16-bit integer (matching the
official terminal), so passwords longer than 127 bytes authenticate correctly
(a signed read would sign-extend the length and break the handshake).

### Concurrent Request Limiting

The SDK caps the number of in-flight market-data requests. The cap
is derived automatically from the account's subscription tier at connect time and is not
user-configurable. This respects the server-side per-tier concurrency limit and prevents
runaway request storms from overwhelming the upstream server or triggering server-side rate
limiting.

### Unknown Compression Rejection

The response decompressor returns an error for unrecognized compression algorithms
instead of silently treating the data as uncompressed. This prevents corrupt data from being
silently passed to callers.

### Streaming Event Dispatch

Streaming uses a dedicated dispatch path with bounded buffering for event
dispatch. The bounded buffer prevents unbounded memory growth from unconsumed
events.

### Frame Size Limits

Binary frame size checks are enforced in release builds, not only in debug
builds. This prevents oversized frames from causing unbounded memory
allocation.

### Dependencies

We review dependencies for:
- Known vulnerabilities (RustSec advisory database)
- License compliance
- Duplicate dependency versions
