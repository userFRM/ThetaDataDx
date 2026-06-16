//! Shared validation for ring-buffer sizing.
//!
//! Ring buffers must be a power of two so the steady-state index wrap
//! reduces to `i & (cap - 1)` — one AND, branchless. A non-power-of-
//! two size forces a modulo (~20 cycles on x86_64) on every consumer
//! iteration, which destroys the instruction-level parallelism the
//! read path relies on.
//!
//! Silent rounding to the next power of two is rejected because it
//! rewrites caller intent. Fail closed at construction time with a
//! diagnostic that names both the offending value and the nearest
//! valid size so the caller can correct the configuration without
//! re-reading the source.

/// Minimum ring buffer size accepted by [`check_ring_size`].
///
/// 64 slots is the minimum for well-formed single-producer single-
/// consumer rings — smaller rings amortise the consumer barrier check
/// across too few events to amortise the cache-line traffic, and
/// larger producers may trip the wrap fence on every fourth publish.
pub const MIN_RING_SIZE: usize = 64;

/// Maximum ring buffer size accepted by [`check_ring_size`].
///
/// The ring is pre-allocated in full at construction, so the size is a
/// memory commitment, not a ceiling that is only reached under load.
/// `2^24` (16,777,216) slots bounds that commitment — at
/// `sizeof(Option<StreamEvent>)` per slot it is already a multi-hundred-
/// megabyte allocation, well above the shipped 131,072-slot default and
/// any realistic burst budget. Without an upper bound an absurd
/// power-of-two (`2^40`, say) passes the power-of-two and minimum checks
/// and the engine attempts to reserve terabytes up front, so the ceiling
/// is enforced and reported by name rather than left to a runtime
/// allocation failure.
pub const MAX_RING_SIZE: usize = 1 << 24;

/// Validate that `n` is a power of two within
/// `[MIN_RING_SIZE, MAX_RING_SIZE]`.
///
/// Returns `Ok(n)` on success; `Err(RingSizeError)` on failure. The
/// error names the offending value and the nearest valid size so the
/// caller can correct the configuration without grep-fishing.
///
/// # Errors
///
/// Returns [`RingSizeError::TooSmall`] when `n` is below
/// [`MIN_RING_SIZE`], [`RingSizeError::TooLarge`] when `n` is above
/// [`MAX_RING_SIZE`], or [`RingSizeError::NotPowerOfTwo`] when `n` is
/// not a power of two.
pub fn check_ring_size(n: usize) -> Result<usize, RingSizeError> {
    if n < MIN_RING_SIZE {
        return Err(RingSizeError::TooSmall {
            provided: n,
            minimum: MIN_RING_SIZE,
        });
    }
    if n > MAX_RING_SIZE {
        return Err(RingSizeError::TooLarge {
            provided: n,
            maximum: MAX_RING_SIZE,
        });
    }
    if !n.is_power_of_two() {
        return Err(RingSizeError::NotPowerOfTwo {
            provided: n,
            suggested: n.next_power_of_two(),
        });
    }
    Ok(n)
}

/// Failures rejected by [`check_ring_size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingSizeError {
    /// `provided` is smaller than [`MIN_RING_SIZE`].
    TooSmall {
        /// The size the caller supplied.
        provided: usize,
        /// The minimum the validator accepts.
        minimum: usize,
    },
    /// `provided` is larger than [`MAX_RING_SIZE`].
    TooLarge {
        /// The size the caller supplied.
        provided: usize,
        /// The maximum the validator accepts.
        maximum: usize,
    },
    /// `provided` is not a power of two. `suggested` is the nearest
    /// valid power of two `>= provided` so the caller can pick the
    /// next viable budget without recomputing it.
    NotPowerOfTwo {
        /// The size the caller supplied.
        provided: usize,
        /// The nearest valid power of two `>= provided`.
        suggested: usize,
    },
}

impl std::fmt::Display for RingSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { provided, minimum } => {
                write!(f, "ring_size {provided} is below the minimum of {minimum}")
            }
            Self::TooLarge { provided, maximum } => {
                write!(f, "ring_size {provided} is above the maximum of {maximum}")
            }
            Self::NotPowerOfTwo {
                provided,
                suggested,
            } => write!(
                f,
                "ring_size {provided} must be a power of two; nearest valid value is {suggested}"
            ),
        }
    }
}

impl std::error::Error for RingSizeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_powers_of_two() {
        assert_eq!(check_ring_size(64), Ok(64));
        assert_eq!(check_ring_size(1024), Ok(1024));
        assert_eq!(check_ring_size(131_072), Ok(131_072));
    }

    #[test]
    fn rejects_non_power_of_two() {
        let err = check_ring_size(65).unwrap_err();
        assert_eq!(
            err,
            RingSizeError::NotPowerOfTwo {
                provided: 65,
                suggested: 128,
            }
        );
    }

    #[test]
    fn rejects_below_minimum() {
        let err = check_ring_size(32).unwrap_err();
        assert_eq!(
            err,
            RingSizeError::TooSmall {
                provided: 32,
                minimum: MIN_RING_SIZE,
            }
        );
    }

    #[test]
    fn error_message_names_offender_and_suggestion() {
        let msg = check_ring_size(1000).unwrap_err().to_string();
        assert!(msg.contains("1000"));
        assert!(msg.contains("1024"));
    }

    #[test]
    fn error_message_names_minimum_on_too_small() {
        let msg = check_ring_size(16).unwrap_err().to_string();
        assert!(msg.contains("16"));
        assert!(msg.contains("64"));
    }

    #[test]
    fn accepts_maximum() {
        assert_eq!(check_ring_size(MAX_RING_SIZE), Ok(MAX_RING_SIZE));
    }

    #[test]
    fn shipped_default_is_under_maximum() {
        // The default shipped in `config.default.toml`.
        assert_eq!(check_ring_size(131_072), Ok(131_072));
        assert!(131_072 < MAX_RING_SIZE);
    }

    #[test]
    fn rejects_above_maximum() {
        // A power of two one step above the ceiling must be rejected
        // before the engine pre-allocates an absurd ring.
        let oversized = MAX_RING_SIZE << 1;
        let err = check_ring_size(oversized).unwrap_err();
        assert_eq!(
            err,
            RingSizeError::TooLarge {
                provided: oversized,
                maximum: MAX_RING_SIZE,
            }
        );
    }

    #[test]
    fn error_message_names_maximum_on_too_large() {
        let oversized = 1usize << 40;
        let msg = check_ring_size(oversized).unwrap_err().to_string();
        assert!(msg.contains(&oversized.to_string()));
        assert!(msg.contains(&MAX_RING_SIZE.to_string()));
    }
}
