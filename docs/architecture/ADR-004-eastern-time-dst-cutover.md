# ADR-004: Eastern-time DST cutover handling

## Status

Accepted.

## Context

ThetaDataDx timestamps are emitted in U.S. Eastern time. Eastern DST rules
changed on 2007-08-08 (effective 2007 calendar year):

- **Pre-2007:** DST started on the first Sunday of April and ended on the
  last Sunday of October.
- **2007-onward:** DST starts on the second Sunday of March and ends on the
  first Sunday of November.

Three surfaces need to apply the same cutover: flatfiles (historical archive
decoding back to 2003), MDDS (live response decoding), and FPSS (real-time
tick wall-clock derivation). Until this ADR, the DST math lived inside
`crates/thetadatadx/src/decode.rs` and was not reachable from flatfiles or
fpss without crate-level cycles.

## Decision

A single canonical implementation lives in `tdbe::time`:

- `eastern_offset_ms(ms_since_unix_epoch: i64) -> i64`
- `march_second_sunday_utc(year)`, `november_first_sunday_utc(year)`
  (post-2007 anchors)
- `april_first_sunday_utc(year)`, `october_last_sunday_utc(year)`
  (pre-2007 anchors)
- `civil_to_epoch_days(year, month, day)` and `timestamp_to_*` helpers

Wave 2 lifts these from `decode.rs` into `tdbe::time` so flatfiles, mdds, and
fpss share the same module without a reverse dependency.

## Alternatives

- **Vendor `chrono-tz` / IANA tzdata.** Rejected: pulls a multi-MB tzdata
  blob into the binary core for a single timezone the SDK hard-codes
  anyway; rebuild cadence drifts independently of upstream Java parity.
- **Duplicate the math per surface.** Rejected: how the bug got introduced
  in the first place. Three implementations means three opportunities for
  silent drift.

## Consequences

- Flatfiles, MDDS, and FPSS share one DST module with one source of truth.
- The 2007 cutover and the pre-2007 / post-2007 fork are tested in `tdbe`
  against a hand-built fixture of NYSE half-day calendars.
- Any future U.S. Congress DST tweak edits one file.
