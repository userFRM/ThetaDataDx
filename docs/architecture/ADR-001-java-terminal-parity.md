# ADR-001: Java terminal parity sourcing

## Status

Accepted.

## Context

ThetaDataDx is reverse-engineered from the official Java terminal. Prior to this
ADR, every Rust module that mirrored a Java class carried inline
`Source: <JavaClass>.method()` breadcrumbs. That repeated metadata leaked
vendor-internal class names into the public source surface and tied per-line
maintenance to upstream binary changes the public crate cannot describe.

## Decision

- Every Rust module whose behaviour mirrors an upstream Java class carries a
  single doc-header link back to this ADR — never per-line breadcrumbs.
- The authoritative reverse-engineering ledger lives at
  `private/JAVA_TERMINAL_FINDINGS.md` and `private/JAVA_TERMINAL_MAPPING.md`.
  Both are gitignored. They map every public Rust entity to the Java class /
  method that informed it.
- Per-line `Source:` breadcrumbs in source files are deleted. The ADR + private
  ledger replace them.

## Alternatives

- **Keep inline breadcrumbs.** Rejected: leaks vendor class names into the
  public source tree; rotates with every upstream binary version.
- **Drop the ledger entirely.** Rejected: reproducibility for new contributors
  evaporates the moment the original author rotates off.

## Consequences

- Public source surface stays clean of vendor class names.
- Reproducibility for the reverse-engineering work is preserved through the
  private ledger.
- New mirroring work updates the private ledger first; the source files stay
  vocabulary-neutral with one ADR-001 link in the module header.
