# ADR-007: Flatfiles MDDS SPKI pin

## Status

Accepted.

## Context

The MDDS flatfiles surface authenticates against a long-lived data-centre
endpoint that, prior to this ADR, was reachable only via a static TLS
handshake. The standard public-CA PKI chain is insufficient: a CA compromise
or a country-level CA injection could MITM a multi-gigabyte historical
download without surfacing any error to the SDK.

To close that window, the flatfiles client pins a specific Subject Public
Key Info (SPKI) value at
`crates/thetadatadx/src/flatfiles/mdds_spki.rs`. Any handshake whose leaf
certificate's SPKI does not match the pinned value is rejected before any
request bytes are sent.

## Decision

- The SPKI pin is hard-coded at the source level, not configurable at
  runtime, so a compromised process cannot relax it.
- Rotating the pin requires a coordinated PR carrying the new SPKI value,
  staged through dev → stage → prod, with the old pin retained as a
  short-window fallback during the cutover.
- The PR itself is the rotation gate; there is no out-of-band override.

## Alternatives

- **Rely on public CA chain only.** Rejected: a CA compromise silently
  breaks data integrity for the most expensive surface (multi-gigabyte
  historical downloads).
- **Runtime-configurable pin.** Rejected: a process compromised at
  runtime can also rewrite the pin, defeating the whole point. A source-
  level pin requires a code release to weaken.

## Consequences

- MITM resistance against CA-level compromise on the flatfiles path.
- Rotation is intentional friction — by design, not by accident.
- A cert rotation that catches the SDK off-guard manifests as a hard
  flatfiles failure, surfaced to the operator immediately rather than
  silently downloading from the wrong endpoint.
