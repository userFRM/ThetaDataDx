# ADR-003: MDDS concurrent-request limit as `2^tier`

## Status

Accepted.

## Context

MDDS subscription tiers (`Free`, `Value`, `Standard`, `Pro`) gate the maximum
number of in-flight requests a single client may hold against the gRPC server.
The historical mapping was Free=1, Value=2, Standard=4, Pro=8 — a `2^tier`
progression. Until this ADR the mapping lived as a magic-number table inside
`mdds/client.rs` with a comment line referencing the four discriminant values
by hand.

## Decision

The mapping is codified at the type level:

```rust
pub fn max_concurrent_requests(self) -> usize { 1usize << self as u32 }
```

`SubscriptionTier` is a typed enum (variants `Free=0`, `Value=1`, `Standard=2`,
`Pro=3`) replacing the previous `Option<i32>` field on the client. Any wire
integer outside `0..=3` is rejected at construction via
`SubscriptionTier::from_wire(i32) -> Option<Self>`.

The Wave 3 PR moves the existing magic-number table into this single shift.

## Alternatives

- **Keep the magic-number table.** Rejected: invites silent drift if upstream
  ever tweaks one tier's limit; the comment is the only spec.
- **Hard-code via `match self { Free => 1, ... }`.** Rejected: hides the
  `2^tier` semantic behind a switch statement. The shift makes the
  doubling-per-tier rule visible at the call site.

## Consequences

- Tier semantics are explicit at the type level instead of in comments.
- Adding a future tier becomes "add a discriminant," with the limit derived
  for free as long as the doubling rule holds.
- A regression that breaks the `2^tier` invariant becomes a one-line fix in
  one place.
