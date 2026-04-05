//! Trade sequence number handling for `ThetaData` FPSS streams.
//!
//! Provides wrapping-aware sequence tracking for i32 trade sequence numbers
//! that overflow from `i32::MAX` to `i32::MIN` and map into a monotonic
//! absolute counter.

/// Maximum raw sequence value before overflow.
pub const SEQUENCE_MAX: i64 = i32::MAX as i64;

/// Minimum raw sequence value (wraps here after overflow).
pub const SEQUENCE_MIN: i64 = i32::MIN as i64;

/// Total number of distinct sequence values in one cycle.
pub const SEQUENCE_RANGE: i64 = (SEQUENCE_MAX - SEQUENCE_MIN) + 1;

/// A single trade sequence number with both raw (wire) and absolute forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeSequence {
    raw: i64,
    absolute: u64,
}

impl TradeSequence {
    /// Create from a raw sequence value (absolute = raw as u64).
    // Reason: protocol-level wrapping for Java i32 sequence interop.
    #[allow(clippy::cast_sign_loss)]
    #[inline]
    #[must_use]
    pub const fn new(raw: i64) -> Self {
        Self {
            raw,
            absolute: raw as u64,
        }
    }

    /// Create with an explicit absolute value.
    #[inline]
    #[must_use]
    pub const fn with_absolute(raw: i64, absolute: u64) -> Self {
        Self { raw, absolute }
    }

    /// The raw (wire-format) sequence number.
    #[inline]
    #[must_use]
    pub const fn raw(&self) -> i64 {
        self.raw
    }

    /// The monotonically-increasing absolute sequence number.
    #[inline]
    #[must_use]
    pub const fn absolute(&self) -> u64 {
        self.absolute
    }

    /// True when the raw value is -1 (one step before overflow to 0).
    #[inline]
    #[must_use]
    pub const fn is_at_overflow(&self) -> bool {
        self.raw == -1
    }

    /// True when this is 0 and the previous was -1 (second-cycle zero).
    #[inline]
    #[must_use]
    pub const fn is_second_zero(&self, previous: &TradeSequence) -> bool {
        self.raw == 0 && previous.raw == -1
    }

    /// Compute the next sequence value, wrapping at `SEQUENCE_MAX`.
    #[inline]
    #[must_use]
    pub const fn next(&self) -> Self {
        let next_raw = if self.raw == SEQUENCE_MAX {
            SEQUENCE_MIN
        } else {
            self.raw + 1
        };
        Self {
            raw: next_raw,
            absolute: self.absolute + 1,
        }
    }

    /// Number of sequence steps from `self` to `other`.
    #[inline]
    #[must_use]
    pub const fn gap_to(&self, other: &TradeSequence) -> u64 {
        other.absolute.saturating_sub(self.absolute)
    }

    /// True if there is a gap (> 1 step) between `self` and `previous`.
    #[inline]
    #[must_use]
    pub const fn has_gap(&self, previous: &TradeSequence) -> bool {
        self.gap_to(previous) > 1 || previous.gap_to(self) > 1
    }

    /// Number of missing messages between `self` and `previous`.
    #[inline]
    #[must_use]
    pub const fn missing_count(&self, previous: &TradeSequence) -> u64 {
        let gap = self.absolute.abs_diff(previous.absolute);
        gap.saturating_sub(1)
    }
}

/// Tracks sequence numbers across a stream, detecting gaps and overflows.
#[derive(Debug, Clone)]
pub struct SequenceTracker {
    last: Option<TradeSequence>,
    overflow_count: u64,
    gap_count: u64,
    missing_messages: u64,
}

impl SequenceTracker {
    /// Create a fresh tracker with no history.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last: None,
            overflow_count: 0,
            gap_count: 0,
            missing_messages: 0,
        }
    }

    /// Process a raw sequence value, returning the update details.
    // Reason: protocol-level wrapping for Java i32 sequence overflow handling.
    #[allow(clippy::cast_sign_loss)]
    pub fn process(&mut self, raw: i64) -> SequenceUpdate {
        let mut is_overflow = false;
        let mut is_gap = false;
        let mut missing_count = 0;

        let sequence = if let Some(last) = self.last {
            let absolute = if raw == 0 && last.raw == -1 {
                is_overflow = true;
                self.overflow_count += 1;
                self.overflow_count * SEQUENCE_RANGE as u64
            } else if raw < 0 && last.raw >= 0 && last.raw >= SEQUENCE_MAX - 1000 {
                is_overflow = true;
                self.overflow_count += 1;
                let base = if self.overflow_count > 0 {
                    (self.overflow_count - 1) * SEQUENCE_RANGE as u64
                } else {
                    0
                };
                base + (SEQUENCE_RANGE + raw) as u64
            } else if raw >= last.raw {
                last.absolute.saturating_add((raw - last.raw) as u64)
            } else {
                let diff = raw - last.raw;
                if diff < 0 {
                    last.absolute
                } else {
                    last.absolute.saturating_add(diff as u64)
                }
            };

            let seq = TradeSequence::with_absolute(raw, absolute);

            if seq.absolute > last.absolute {
                let gap = seq.absolute - last.absolute;
                if gap > 1 {
                    is_gap = true;
                    missing_count = gap - 1;
                    self.gap_count += 1;
                    self.missing_messages += missing_count;
                }
            }

            seq
        } else {
            TradeSequence::new(raw)
        };

        self.last = Some(sequence);

        SequenceUpdate {
            sequence,
            is_overflow,
            is_gap,
            missing_count,
        }
    }

    /// The last processed sequence, if any.
    #[inline]
    #[must_use]
    pub const fn last(&self) -> Option<&TradeSequence> {
        match &self.last {
            Some(seq) => Some(seq),
            None => None,
        }
    }

    /// Total number of overflow events detected.
    #[inline]
    #[must_use]
    pub const fn overflow_count(&self) -> u64 {
        self.overflow_count
    }

    /// Total number of gap events detected.
    #[inline]
    #[must_use]
    pub const fn gap_count(&self) -> u64 {
        self.gap_count
    }

    /// Total number of missing messages across all gaps.
    #[inline]
    #[must_use]
    pub const fn missing_messages(&self) -> u64 {
        self.missing_messages
    }

    /// Reset all state.
    #[inline]
    pub fn reset(&mut self) {
        self.last = None;
        self.overflow_count = 0;
        self.gap_count = 0;
        self.missing_messages = 0;
    }
}

impl Default for SequenceTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of processing a single sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceUpdate {
    pub sequence: TradeSequence,
    pub is_overflow: bool,
    pub is_gap: bool,
    pub missing_count: u64,
}

/// Convert a signed raw sequence to an unsigned absolute value.
// Reason: protocol-level wrapping for Java i32 sequence interop.
#[allow(clippy::cast_sign_loss)]
#[inline]
#[must_use]
pub const fn signed_to_unsigned(signed: i64) -> u64 {
    if signed >= 0 {
        signed as u64
    } else {
        (SEQUENCE_RANGE + signed) as u64
    }
}

/// Convert an unsigned absolute value back to a signed raw sequence.
// Reason: protocol-level wrapping for Java i32 sequence interop.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
#[inline]
#[must_use]
pub const fn unsigned_to_signed(unsigned: u64) -> i64 {
    if unsigned <= SEQUENCE_MAX as u64 {
        unsigned as i64
    } else {
        (unsigned as i64) - SEQUENCE_RANGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_new() {
        let seq = TradeSequence::new(42);
        assert_eq!(seq.raw(), 42);
        assert_eq!(seq.absolute(), 42);
    }

    #[test]
    fn sequence_next() {
        let seq = TradeSequence::new(100);
        let next = seq.next();
        assert_eq!(next.raw(), 101);
        assert_eq!(next.absolute(), 101);
    }

    #[test]
    fn sequence_overflow_at_max() {
        let at_max = TradeSequence::new(SEQUENCE_MAX);
        let after_max = at_max.next();
        assert_eq!(after_max.raw(), SEQUENCE_MIN);
        assert_eq!(after_max.absolute(), (SEQUENCE_MAX as u64) + 1);
    }

    #[test]
    fn signed_unsigned_roundtrip() {
        for signed in [-2_147_483_648i64, -1, 0, 1, 2_147_483_647] {
            let unsigned = signed_to_unsigned(signed);
            let back = unsigned_to_signed(unsigned);
            assert_eq!(signed, back);
        }
    }

    #[test]
    fn tracker_gap_detection() {
        let mut tracker = SequenceTracker::new();
        tracker.process(0);
        tracker.process(1);
        let update = tracker.process(5);
        assert!(update.is_gap);
        assert_eq!(update.missing_count, 3);
        assert_eq!(tracker.gap_count(), 1);
        assert_eq!(tracker.missing_messages(), 3);
    }

    #[test]
    fn tracker_overflow_detection() {
        let mut tracker = SequenceTracker::new();
        tracker.process(SEQUENCE_MAX - 1);
        tracker.process(SEQUENCE_MAX);
        let update = tracker.process(SEQUENCE_MIN);
        assert!(update.is_overflow);
        assert_eq!(tracker.overflow_count(), 1);
    }

    #[test]
    fn tracker_reset() {
        let mut tracker = SequenceTracker::new();
        tracker.process(0);
        tracker.process(5);
        assert_eq!(tracker.gap_count(), 1);
        tracker.reset();
        assert_eq!(tracker.gap_count(), 0);
        assert_eq!(tracker.missing_messages(), 0);
        assert!(tracker.last().is_none());
    }
}
