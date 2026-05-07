# ADR-002: FPSS ring buffer power-of-two capacity

## Status

Accepted.

## Context

`crates/thetadatadx/src/fpss/ring.rs` provides the single-producer / single-
consumer ring buffer that bridges the FPSS read loop and the streaming
dispatcher. Hot-path indexing happens on every tick.

A ring whose capacity is a power of two can wrap an index with `i & (cap - 1)`
— a single AND, branchless. A capacity that is not a power of two requires a
modulo against a non-constant divisor (~20 cycles on x86_64) or a comparison-
plus-conditional-subtract pair, both of which break the steady-state ILP the
read path relies on.

## Decision

Ring buffer construction enforces `capacity.is_power_of_two()`. Anything else
returns an error at construction; there is no silent rounding. The invariant
is documented at the type and re-asserted in the constructor body so removing
the check leaves a compile-time tombstone.

## Alternatives

- **Round up to the next power of two on construction.** Rejected: silently
  changes the caller's stated buffer budget. Bloomberg-grade APIs do not
  silently rewrite caller intent.
- **Use modulo on every wrap.** Rejected: ~20 cycles per tick on the read
  path is unacceptable on the steady-state quote feed.

## Consequences

- Index wrap is one AND, branchless, on every read and write.
- Non-power-of-two sizes are a hard error at construction; callers see the
  failure immediately rather than discovering a perf cliff under load.
- The ring's documented capacity equals what the caller passed in.
