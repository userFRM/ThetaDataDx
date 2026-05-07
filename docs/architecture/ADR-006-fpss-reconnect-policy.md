# ADR-006: FPSS reconnect policy

## Status

Accepted.

## Context

The FPSS streaming surface holds a long-lived TLS socket against the upstream
data centre. Disconnects come in two flavours:

1. **Generic transient drops** (network blip, TLS renegotiation, peer
   restart). The right behaviour is a short backoff and retry.
2. **Rate-limited disconnects** (HTTP-429-equivalent on the FPSS handshake,
   or the upstream's per-account concurrent-session limit). The right
   behaviour is a long enough wait that a reconnect storm cannot escalate
   the rate-limit window.

Hammering the upstream after a 429 burns the account's reconnect budget and
ends with a multi-minute lockout. Hammering a transient drop, on the other
hand, recovers the stream in single-digit seconds.

## Decision

`ReconnectPolicy` exposes two timings and an attempt cap:

- `wait_ms` — default backoff for generic disconnects.
- `wait_rate_limited_ms` — 130 seconds, used only after a rate-limit
  signal from the FPSS handshake.
- Maximum reconnect attempts capped at 5 before the dispatcher surfaces a
  hard failure to the caller.

The rate-limit wait is intentionally larger than the upstream's published
rate-limit window so a single reconnect storm cannot bracket a second
window.

## Alternatives

- **Single backoff for all disconnect causes.** Rejected: a generic
  exponential backoff long enough to absorb a 429 is unacceptable for
  transient drops; one short enough for transient drops re-triggers the 429.
- **Unbounded retries.** Rejected: a permanent upstream issue would burn
  the reconnect budget silently and never surface to the operator.

## Consequences

- Rate-limited bursts back off without runaway reconnect storms.
- Transient drops recover in `wait_ms` rather than `wait_rate_limited_ms`.
- Hard failures surface to the caller after 5 attempts; no silent stalls.
