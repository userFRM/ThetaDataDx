//! Cache-line padding for concurrently-written atomics.
//!
//! Shared by any subsystem that places two atomics written by
//! different threads side by side: padding each value to its own
//! cache line keeps one writer's line ownership from evicting the
//! other writer's line (false sharing).

/// Pad `T` to one cache line so concurrent writes to distinct counters
/// do not false-share. 128 bytes covers both x86_64 (64-byte line) and
/// heterogeneous ARM cores (up to 128-byte line pairs).
#[derive(Debug)]
#[repr(C, align(128))]
pub(crate) struct CachePadded<T>(T);

impl<T> CachePadded<T> {
    #[inline]
    pub(crate) const fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T> std::ops::Deref for CachePadded<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// The padding contract: each padded value owns at least one full
    /// 128-byte alignment unit, so two adjacent fields can never share
    /// a cache line on any supported target.
    #[test]
    fn padded_atomic_is_full_line_sized_and_aligned() {
        assert_eq!(std::mem::align_of::<CachePadded<AtomicI64>>(), 128);
        assert_eq!(std::mem::size_of::<CachePadded<AtomicI64>>(), 128);
    }

    #[test]
    fn deref_reaches_the_inner_value() {
        let cell = CachePadded::new(AtomicI64::new(-1));
        assert_eq!(cell.load(Ordering::Relaxed), -1);
        cell.store(42, Ordering::Relaxed);
        assert_eq!(cell.load(Ordering::Relaxed), 42);
    }
}
