# ADR-005: OCC-21 contract ID century scope

## Status

Accepted.

## Context

OCC-21 contract identifiers (the 21-character symbology used by the OCC and
mirrored on the FPSS wire) encode the option's expiration year as a two-digit
field `YY`. There is no century qualifier on the wire.

The SDK has to pick a century interpretation. U.S. listed options expire on
the third Friday of the contract month, with LEAPS extending up to ~3 years
out. Today (2026) the longest live LEAPS expire in 2028. OCC has never issued
options with expirations later than 2099-12-31, and there is no proposed
cutover to a three-digit year field.

## Decision

OCC-21 parsing inside `Contract::parse_occ21` interprets `YY` as `2000 + YY`.
The accepted century is `2000-2099`; expirations outside this range are
rejected at the input boundary with a structured error.

## Alternatives

- **Sliding 50-year window.** Rejected: the wire format is OCC's, OCC has
  not published a sliding-window rule, and a sliding window in the SDK
  would silently disagree with OCC's own systems for far-out LEAPS.
- **Accept any `YY` and round.** Rejected: silent rounding violates the
  Bloomberg-grade rule that data parsers do not invent input.

## Consequences

- Inputs outside 2000-2099 are rejected at the parse boundary; no silent
  reinterpretation reaches FPSS.
- OCC-21 parsing is a pure function of the wire bytes, no clock dependency.

## When this exception expires

2099-12-31. If OCC-21 outlives this ADR, this file is the spec for the
follow-up decision.
