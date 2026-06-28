//! Trade sequence number handling for `ThetaData` streaming.
//!
//! Maps i32 trade sequence numbers that overflow from `i32::MAX` to
//! `i32::MIN` onto a contiguous unsigned axis.

/// Maximum raw sequence value before overflow.
pub const SEQUENCE_MAX: i64 = i32::MAX as i64;

/// Minimum raw sequence value (wraps here after overflow).
pub const SEQUENCE_MIN: i64 = i32::MIN as i64;

/// Total number of distinct sequence values in one cycle.
pub const SEQUENCE_RANGE: i64 = (SEQUENCE_MAX - SEQUENCE_MIN) + 1;

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
    fn signed_unsigned_roundtrip() {
        for signed in [-2_147_483_648i64, -1, 0, 1, 2_147_483_647] {
            let unsigned = signed_to_unsigned(signed);
            let back = unsigned_to_signed(unsigned);
            assert_eq!(signed, back);
        }
    }
}
