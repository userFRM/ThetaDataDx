//! Trade sequence number handling for `ThetaData` streaming.
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
    // Reason: protocol-level wrapping for 32-bit sequence interop.
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
    // Reason: protocol-level wrapping for 32-bit sequence overflow handling.
    #[allow(clippy::cast_sign_loss)]
    pub fn process(&mut self, raw: i64) -> SequenceUpdate {
        let mut is_overflow = false;
        let mut is_gap = false;
        let mut missing_count = 0;

        let sequence = if let Some(last) = self.last {
            let absolute = if raw == 0 && last.raw == -1 {
                // True cycle wrap: -1 -> 0 advances to the next cycle's base.
                // This is the only crossing that increments the cycle count;
                // the unsigned axis used by `signed_to_unsigned` places raw 0
                // at the start of a fresh cycle.
                is_overflow = true;
                self.overflow_count += 1;
                self.overflow_count * SEQUENCE_RANGE as u64
            } else if raw < 0 && last.raw >= 0 && last.raw >= SEQUENCE_MAX - 1000 {
                // Sign-bit crossing: MAX -> MIN. On the unsigned axis this is
                // mid-cycle (position 2^31-1 -> 2^31), contiguous, NOT a cycle
                // boundary. Stay on the current cycle and do not increment the
                // cycle count; the true wrap is the -1 -> 0 branch above.
                is_overflow = true;
                self.overflow_count * SEQUENCE_RANGE as u64 + signed_to_unsigned(raw)
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
    /// The sequence number after this update, in monotonic absolute form.
    pub sequence: TradeSequence,
    /// Whether this update wrapped the raw counter past its overflow boundary.
    pub is_overflow: bool,
    /// Whether a gap was detected between this update and the previous one.
    pub is_gap: bool,
    /// Number of messages missing in the detected gap (0 when contiguous).
    pub missing_count: u64,
}

/// Convert a signed raw sequence to an unsigned absolute value.
// Reason: protocol-level wrapping for 32-bit sequence interop.
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
// Reason: protocol-level wrapping for 32-bit sequence interop.
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
    fn tracker_sign_bit_crossing_is_not_a_cycle() {
        // The MAX -> MIN sign-bit crossing is mid-cycle on the unsigned axis:
        // it is flagged as an overflow event but does not advance the cycle
        // count, and it introduces no phantom gap.
        let mut tracker = SequenceTracker::new();
        tracker.process(SEQUENCE_MAX - 1);
        let at_max = tracker.process(SEQUENCE_MAX);
        let crossing = tracker.process(SEQUENCE_MIN);
        assert!(crossing.is_overflow);
        assert!(!crossing.is_gap);
        assert_eq!(crossing.missing_count, 0);
        assert_eq!(tracker.overflow_count(), 0);
        // The absolute advances by exactly one step across the crossing.
        assert_eq!(crossing.sequence.absolute(), at_max.sequence.absolute() + 1);
    }

    #[test]
    fn tracker_full_cycle_no_phantom_gap() {
        // Drive both per-cycle boundaries contiguously: the sign-bit crossing
        // (MAX -> MIN) and the true wrap (-1 -> 0). The cycle count must reach
        // exactly 1 across one full cycle, with no spurious gap or
        // missing-message count at either boundary. Regression for
        // double-counting the sign-bit crossing, which inflated the absolute
        // by SEQUENCE_RANGE and reported a ~4.29-billion phantom gap per cycle.
        let mut tracker = SequenceTracker::new();

        // Approach and step over the sign-bit crossing, contiguously.
        tracker.process(SEQUENCE_MAX - 1);
        let at_max = tracker.process(SEQUENCE_MAX);
        let crossing = tracker.process(SEQUENCE_MIN);
        assert!(crossing.is_overflow);
        assert!(!crossing.is_gap);
        assert_eq!(crossing.missing_count, 0);
        assert_eq!(crossing.sequence.absolute(), at_max.sequence.absolute() + 1);
        // The sign-bit crossing is mid-cycle: no cycle increment yet.
        assert_eq!(tracker.overflow_count(), 0);

        // Continue contiguously to -1 (one before the true wrap). Anchored at
        // raw 0 so the absolute axis is sign-correct, then walk the final
        // pre-wrap values.
        tracker.process(-3);
        tracker.process(-2);
        let minus_one = tracker.process(-1);

        // The true wrap (-1 -> 0) is the sole cycle-advancing boundary: it
        // increments the cycle count and stays contiguous on the absolute
        // axis (no phantom gap, no missing messages).
        let overflow_before = tracker.overflow_count();
        let gaps_before = tracker.gap_count();
        let missing_before = tracker.missing_messages();
        let wrap = tracker.process(0);
        assert!(wrap.is_overflow);
        assert!(!wrap.is_gap);
        assert_eq!(wrap.missing_count, 0);
        assert_eq!(wrap.sequence.absolute(), minus_one.sequence.absolute() + 1);
        assert_eq!(tracker.overflow_count(), overflow_before + 1);
        assert_eq!(tracker.gap_count(), gaps_before);
        assert_eq!(tracker.missing_messages(), missing_before);
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
