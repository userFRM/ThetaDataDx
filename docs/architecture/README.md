# Architecture decisions

This directory holds the Architecture Decision Records (ADRs) that
document binding architectural choices in the SDK.

| ADR                                                  | Topic                                              |
| ---------------------------------------------------- | -------------------------------------------------- |
| [ADR-001](ADR-001-java-terminal-parity.md)           | Java terminal parity reverse-engineering source    |
| [ADR-002](ADR-002-fpss-ring-power-of-two.md)         | FPSS Disruptor ring sizing policy                  |
| [ADR-003](ADR-003-mdds-semaphore-pow2-tier.md)       | MDDS gRPC concurrency = `2^subscription_tier`      |
| [ADR-004](ADR-004-eastern-time-dst-cutover.md)       | DST-aware Eastern-Time cutover handling            |
| [ADR-005](ADR-005-occ21-century-scope.md)            | OCC-21 contract identifier century scope           |
| [ADR-006](ADR-006-fpss-reconnect-policy.md)          | FPSS auto-reconnect policy                         |
| [ADR-007](ADR-007-flatfiles-mdds-spki-pin.md)        | Flatfiles + MDDS SPKI certificate pin              |

## Notes on prior planning documents

v9.0.0 shipped the public-API redesign that was previously sketched in
`docs/public-api-redesign.md` (now removed): the two-layer exact +
ergonomic surface, typed scalars (`Date`, `Year`, `Right`,
`Expiration`, `Strike`, `TimeOfDay`, `Interval`), `IntoOptionSpec`,
`for_contracts(...)`, and `StrikeSelector` / `ExpirationSelector`
wildcards are all live on the `9.0.x` line. New architectural decisions
land here as ADRs; planning prose for shipped surfaces is intentionally
not preserved.
