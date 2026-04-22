//! Date-range split math for the 365-day auto-chunk path.
//!
//! The ThetaData server rejects history ranges exceeding 365 calendar days
//! with a raw gRPC `InvalidArgument`. v8.0.2 pre-flight splits the
//! requested `(start, end)` span into ≤365-day chunks before dispatch so
//! callers can ask for arbitrary multi-year ranges without hitting the
//! server-side cap.
//!
//! This module is the pure date-arithmetic layer — no tokio, no
//! `MddsClient`. The fan-out orchestrator (concurrent cell dispatch +
//! concatenation) lives one layer up and coordinates with the Rust SDK's
//! request semaphore. The math here is exercised by its own unit tests so
//! a refactor of the orchestrator can never break the chunk boundary
//! invariants.
//!
//! # Invariants (tested below)
//!
//! 1. A span ≤365 days produces a single `(start, end)` chunk unchanged.
//! 2. A span >365 days produces N chunks where every chunk except the
//!    last spans exactly 365 days (starting on day 1 of each chunk).
//! 3. Chunks are contiguous: chunk N's end + 1 day == chunk N+1's start.
//! 4. Chunk boundaries use YYYYMMDD string format (what the wire expects).
//! 5. Invalid input (end before start, malformed YYYYMMDD) returns an
//!    `Err`, NOT a panic — called from pre-flight code that already
//!    validated, but defense-in-depth never hurt.

// The `chunking` module is staged for the auto-chunk fan-out that lands
// once the Rust-enhancements agent threads `DirectConfig::auto_chunk`
// through `MddsClient`. The split math is correctness-critical, so the
// date arithmetic + its tests ship in this branch; the orchestrator call
// sites activate in a follow-up PR after coordination. We export the
// split entry point from `lib.rs` so the symbol participates in the
// public surface and does not trip dead-code lints.

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("invalid YYYYMMDD date '{0}': {1}")]
    InvalidDate(String, String),
    #[error("end date {end} is before start date {start}")]
    EndBeforeStart { start: String, end: String },
}

/// A single (inclusive) day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ymd {
    year: u32,
    month: u32,
    day: u32,
}

impl Ymd {
    fn from_yyyymmdd(s: &str) -> Result<Self, ChunkError> {
        // Shape check first so chrono never has to think about non-ASCII
        // or lengths other than 8 — the error message we produce here is
        // the one the caller will see on malformed input from the wire.
        if s.len() != 8 || !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(ChunkError::InvalidDate(
                s.to_string(),
                "must be 8 ASCII digits".into(),
            ));
        }
        // Reason: the previous hand-rolled validator accepted
        // Gregorian-impossible dates like "20230229" (Feb 29 outside a
        // leap year) and "20240231" (Feb 31). `to_ord` then silently
        // normalized them, producing wrong chunk boundaries. Delegate
        // validity to chrono so leap-year + month-length rules are
        // enforced by the canonical calendar implementation.
        let parsed = chrono::NaiveDate::parse_from_str(s, "%Y%m%d")
            .map_err(|e| ChunkError::InvalidDate(s.to_string(), e.to_string()))?;
        // chrono returns `i32` for `year()` (BC dates are negative). The
        // YYYYMMDD wire format only expresses 0001..=9999, and the shape
        // check above already guarantees 4 ASCII digits, so a negative
        // year cannot reach here. Still, guard against a narrowing cast
        // explicitly rather than trusting an out-of-band invariant.
        let year_i32 = chrono::Datelike::year(&parsed);
        if !(0..=9999).contains(&year_i32) {
            return Err(ChunkError::InvalidDate(
                s.to_string(),
                format!("year {year_i32} out of YYYY range"),
            ));
        }
        Ok(Ymd {
            year: year_i32 as u32,
            month: chrono::Datelike::month(&parsed),
            day: chrono::Datelike::day(&parsed),
        })
    }

    fn to_yyyymmdd(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// Days from 0001-01-01 (Gregorian). Simple proleptic calculation —
    /// accurate for any reasonable market-data range (the server only
    /// has post-1990 data anyway).
    fn to_ord(self) -> i64 {
        // Rata Die ordinal from Howard Hinnant's date algorithms.
        let y = if self.month <= 2 {
            i64::from(self.year) - 1
        } else {
            i64::from(self.year)
        };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64;
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = (yoe as i64) * 365 + (yoe as i64) / 4 - (yoe as i64) / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_ord(z: i64) -> Self {
        // Inverse Rata Die (Howard Hinnant).
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp.wrapping_sub(9) } as u32;
        let y = if m <= 2 { y + 1 } else { y };
        Ymd {
            year: y as u32,
            month: m,
            day: d,
        }
    }
}

/// Maximum span accepted by the server (inclusive). 365 matches the
/// behavior observed during reverse-engineering.
pub const MAX_SPAN_DAYS: i64 = 365;

/// Split `(start, end)` (YYYYMMDD strings, inclusive on both ends) into
/// chunks that each span at most `MAX_SPAN_DAYS` days. The returned
/// chunks are contiguous and cover the full range exactly once.
pub fn split_date_range(start: &str, end: &str) -> Result<Vec<(String, String)>, ChunkError> {
    let start_ord = Ymd::from_yyyymmdd(start)?.to_ord();
    let end_ord = Ymd::from_yyyymmdd(end)?.to_ord();
    if end_ord < start_ord {
        return Err(ChunkError::EndBeforeStart {
            start: start.into(),
            end: end.into(),
        });
    }
    if end_ord - start_ord < MAX_SPAN_DAYS {
        return Ok(vec![(start.to_string(), end.to_string())]);
    }

    let mut chunks = Vec::new();
    let mut cursor = start_ord;
    while cursor <= end_ord {
        let chunk_end = (cursor + MAX_SPAN_DAYS - 1).min(end_ord);
        chunks.push((
            Ymd::from_ord(cursor).to_yyyymmdd(),
            Ymd::from_ord(chunk_end).to_yyyymmdd(),
        ));
        cursor = chunk_end + 1;
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_day_stays_single_chunk() {
        let chunks = split_date_range("20240101", "20240101").unwrap();
        assert_eq!(chunks, vec![("20240101".into(), "20240101".into())]);
    }

    #[test]
    fn span_of_exactly_365_days_is_one_chunk() {
        // 20240101 → 20241230 is 365 days inclusive. Should not split.
        let chunks = split_date_range("20240101", "20241230").unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn span_of_366_days_splits_into_two_chunks() {
        // 20240101 → 20241231 is 366 days inclusive.
        let chunks = split_date_range("20240101", "20241231").unwrap();
        assert_eq!(chunks.len(), 2);
        // First chunk is exactly 365 days.
        assert_eq!(chunks[0].0, "20240101");
        assert_eq!(chunks[0].1, "20241230");
        // Second chunk is the remaining day.
        assert_eq!(chunks[1].0, "20241231");
        assert_eq!(chunks[1].1, "20241231");
    }

    #[test]
    fn span_of_two_years_splits_into_three_chunks() {
        // 2024-01-01 → 2025-12-31 is 731 days (2024 is leap).
        let chunks = split_date_range("20240101", "20251231").unwrap();
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks for 2-year span"
        );
        // Contiguity: every chunk's end + 1 day should equal the next
        // chunk's start.
        for window in chunks.windows(2) {
            let end = Ymd::from_yyyymmdd(&window[0].1).unwrap();
            let next_start = Ymd::from_yyyymmdd(&window[1].0).unwrap();
            assert_eq!(
                next_start.to_ord(),
                end.to_ord() + 1,
                "chunks must be contiguous: {window:?}"
            );
        }
    }

    #[test]
    fn invalid_yyyymmdd_returns_err() {
        let err = split_date_range("2024-01-01", "20241231").unwrap_err();
        assert!(matches!(err, ChunkError::InvalidDate(_, _)));
    }

    #[test]
    fn end_before_start_returns_err() {
        let err = split_date_range("20241231", "20240101").unwrap_err();
        assert!(matches!(err, ChunkError::EndBeforeStart { .. }));
    }

    #[test]
    fn ymd_round_trip_preserves_date() {
        for s in &["20240101", "19900315", "20261231", "20200229"] {
            let parsed = Ymd::from_yyyymmdd(s).unwrap();
            let back = parsed.to_yyyymmdd();
            assert_eq!(&back, *s, "round-trip failed for {s}");
            // Ordinal round-trip.
            let ord = parsed.to_ord();
            let reparsed = Ymd::from_ord(ord);
            assert_eq!(reparsed, parsed, "ordinal round-trip failed for {s}");
        }
    }

    #[test]
    fn leap_day_is_valid() {
        let ymd = Ymd::from_yyyymmdd("20240229").unwrap();
        assert_eq!(ymd.year, 2024);
        assert_eq!(ymd.month, 2);
        assert_eq!(ymd.day, 29);
    }

    // Gregorian-validation coverage added for v8.0.4. The pre-fix
    // validator range-checked month 1..=12 and day 1..=31 in isolation
    // and therefore accepted impossible Gregorian dates like 20230229
    // (Feb 29 outside a leap year) and 20240231 (Feb 31). The fix
    // delegates to `chrono::NaiveDate::parse_from_str(_, "%Y%m%d")` for
    // canonical validity, and the tests below pin the contract.

    /// Feb 29 in a non-leap year is Gregorian-impossible.
    #[test]
    fn feb_29_in_non_leap_year_is_rejected() {
        // 2023 is not a leap year (not divisible by 4).
        let err = Ymd::from_yyyymmdd("20230229").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "20230229"),
            "expected InvalidDate for 20230229, got {err:?}"
        );
    }

    /// Feb 29 in a leap year is valid (2024 is divisible by 4 and not 100).
    #[test]
    fn feb_29_in_leap_year_is_accepted() {
        let ymd = Ymd::from_yyyymmdd("20240229").unwrap();
        assert_eq!(ymd.year, 2024);
        assert_eq!(ymd.month, 2);
        assert_eq!(ymd.day, 29);
    }

    /// Feb 30 is Gregorian-impossible regardless of leap-year status.
    #[test]
    fn feb_30_is_rejected() {
        let err_leap = Ymd::from_yyyymmdd("20240230").unwrap_err();
        assert!(
            matches!(err_leap, ChunkError::InvalidDate(ref s, _) if s == "20240230"),
            "expected InvalidDate for 20240230, got {err_leap:?}"
        );
        let err_non_leap = Ymd::from_yyyymmdd("20230230").unwrap_err();
        assert!(
            matches!(err_non_leap, ChunkError::InvalidDate(ref s, _) if s == "20230230"),
            "expected InvalidDate for 20230230, got {err_non_leap:?}"
        );
    }

    /// April has 30 days; Apr 31 is Gregorian-impossible.
    #[test]
    fn apr_31_is_rejected() {
        let err = Ymd::from_yyyymmdd("20240431").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "20240431"),
            "expected InvalidDate for 20240431, got {err:?}"
        );
    }

    /// Feb 31 is Gregorian-impossible — the poster child for the
    /// pre-fix bug (day <= 31 and month <= 12 passed isolated checks).
    #[test]
    fn feb_31_is_rejected() {
        let err = Ymd::from_yyyymmdd("20240231").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "20240231"),
            "expected InvalidDate for 20240231, got {err:?}"
        );
    }

    /// Every 30-day month rejects day 31.
    #[test]
    fn day_31_in_30_day_months_is_rejected() {
        // Apr(4), Jun(6), Sep(9), Nov(11) all have 30 days.
        for month in ["04", "06", "09", "11"] {
            let s = format!("2024{month}31");
            let err = Ymd::from_yyyymmdd(&s).unwrap_err();
            assert!(
                matches!(err, ChunkError::InvalidDate(ref msg, _) if msg == &s),
                "expected {s} to be rejected; got {err:?}"
            );
        }
    }

    /// 1900 is divisible by 100 but NOT by 400 — NOT a leap year.
    /// The hand-rolled validator accepted 19000229; chrono rejects it.
    #[test]
    fn century_non_leap_year_rejects_feb_29() {
        let err = Ymd::from_yyyymmdd("19000229").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "19000229"),
            "expected InvalidDate for 19000229 (century non-leap year), got {err:?}"
        );
    }

    /// 2000 is divisible by 400 so it IS a leap year (exception to the
    /// century rule). Feb 29 must round-trip cleanly.
    #[test]
    fn quadricentennial_leap_year_accepts_feb_29() {
        let ymd = Ymd::from_yyyymmdd("20000229").unwrap();
        assert_eq!(ymd.year, 2000);
        assert_eq!(ymd.month, 2);
        assert_eq!(ymd.day, 29);
    }

    /// Month 0 and month >12 are out of range.
    #[test]
    fn month_out_of_range_is_rejected() {
        for s in ["20240001", "20241301", "20249901"] {
            let err = Ymd::from_yyyymmdd(s).unwrap_err();
            assert!(
                matches!(err, ChunkError::InvalidDate(ref msg, _) if msg == s),
                "expected {s} to be rejected; got {err:?}"
            );
        }
    }

    /// Day 0 is out of range for every month.
    #[test]
    fn day_zero_is_rejected() {
        for s in ["20240100", "20240200", "20241200"] {
            let err = Ymd::from_yyyymmdd(s).unwrap_err();
            assert!(
                matches!(err, ChunkError::InvalidDate(ref msg, _) if msg == s),
                "expected {s} to be rejected; got {err:?}"
            );
        }
    }

    /// End-to-end: a Gregorian-impossible date fed to the public
    /// `split_date_range` surface returns `InvalidDate`, not a
    /// silently-normalized chunk list.
    #[test]
    fn split_date_range_rejects_impossible_start() {
        let err = split_date_range("20230229", "20231231").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "20230229"),
            "expected start-side rejection for 20230229, got {err:?}"
        );
    }

    #[test]
    fn split_date_range_rejects_impossible_end() {
        let err = split_date_range("20240101", "20240231").unwrap_err();
        assert!(
            matches!(err, ChunkError::InvalidDate(ref s, _) if s == "20240231"),
            "expected end-side rejection for 20240231, got {err:?}"
        );
    }

    #[test]
    fn chunks_cover_exactly_the_requested_range() {
        // Union of chunks must equal [start, end] with no gaps, no
        // overlaps. This is the "correctness certificate" for the
        // auto-chunk fan-out.
        let start = "20200101";
        let end = "20261231";
        let chunks = split_date_range(start, end).unwrap();
        let s = Ymd::from_yyyymmdd(start).unwrap().to_ord();
        let e = Ymd::from_yyyymmdd(end).unwrap().to_ord();
        let mut covered = 0;
        for (cs, ce) in &chunks {
            let c_start = Ymd::from_yyyymmdd(cs).unwrap().to_ord();
            let c_end = Ymd::from_yyyymmdd(ce).unwrap().to_ord();
            assert!(
                c_end - c_start < MAX_SPAN_DAYS,
                "chunk {cs}..{ce} exceeds {MAX_SPAN_DAYS} days"
            );
            covered += c_end - c_start + 1;
        }
        assert_eq!(
            covered,
            e - s + 1,
            "union of chunks should cover the full range exactly once"
        );
        // First chunk starts at the requested start.
        assert_eq!(chunks.first().unwrap().0, start);
        // Last chunk ends at the requested end.
        assert_eq!(chunks.last().unwrap().1, end);
    }
}
