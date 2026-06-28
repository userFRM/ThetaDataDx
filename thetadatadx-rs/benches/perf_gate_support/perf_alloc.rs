//! Counting global allocator + deterministic-metric JSON writer shared
//! by the per-binding performance-gate harnesses.
//!
//! Wired into a bench binary via
//! `#[path = "perf_gate_support/perf_alloc.rs"] mod perf_alloc;` plus a
//! `#[global_allocator] static A: perf_alloc::CountingAllocator = ...;`
//! line in that binary. The allocator forwards every request to
//! `std::alloc::System` and tallies the global allocation count in a
//! `Relaxed` atomic. It exists ONLY in bench/test targets — the shipped
//! `thetadatadx` rlib never installs a custom `#[global_allocator]`, so
//! the library's allocator is unchanged.
//!
//! The metric this enables — allocations per decoded row — is fully
//! CPU-independent: the same fixture decoded on a 2-vCPU shared runner
//! and on a developer workstation produces the identical allocation
//! count. That is the property that lets the gate run on heterogeneous
//! CI hardware without flaking, where a wall-clock baseline could not.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

// ─── Counting allocator ──────────────────────────────────────────────
//
// Tracks total allocation count and total bytes allocated, process-wide.
// A measurement brackets the region of interest with two `snapshot()`
// reads and divides the delta by the row count it drove through the
// decode path — so allocator traffic from Criterion bookkeeping, warmup,
// or fixture construction outside the bracket never enters the metric.

/// Process-wide allocation counter. Incremented on every successful
/// `alloc`; never reset by the allocator itself (callers snapshot and
/// diff instead, so concurrent fixture setup cannot corrupt a bracket).
pub static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

/// Process-wide byte counter, kept alongside the count so a metric can
/// report bytes/row for context even though the gate keys on the count.
pub static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);

/// System-allocator shim that tallies allocation count + bytes.
pub struct CountingAllocator;

// SAFETY: every method forwards verbatim to `std::alloc::System`, which
// itself satisfies the `GlobalAlloc` contract (pointer provenance,
// layout honoured on dealloc, no over-alignment over-promises). The
// `Relaxed` atomic adds are pure observational state and cannot violate
// allocator invariants. Bench/test-only; never linked into the shipped
// library.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `GlobalAlloc::alloc` requires `layout.size() > 0` and a
        // power-of-two `layout.align()`; the alloc shim Rust generates
        // for any `#[global_allocator]` enforces both before this call.
        // Forwarding to the `System` impl satisfies it verbatim.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            BYTES_ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `GlobalAlloc::dealloc` requires that `ptr` came from a
        // prior `alloc` on this allocator with the same `layout` and has
        // not yet been freed. The shim Rust generates from `Vec`, `Box`,
        // etc. upholds that pairing; forwarding to `System.dealloc`
        // satisfies the `System` impl.
        unsafe { System.dealloc(ptr, layout) };
    }
}

/// `(alloc_count, bytes_allocated)` read of the global counters.
#[inline]
#[must_use]
pub fn snapshot() -> (u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        BYTES_ALLOCATED.load(Ordering::Relaxed),
    )
}

/// One deterministic metric sample: a stable id, the allocation count
/// observed per unit (per decoded row, or per FFI call), the bytes/unit
/// for context, and the unit count the bracket drove.
#[derive(Clone)]
pub struct Sample {
    pub id: String,
    pub allocs_per_unit: f64,
    pub bytes_per_unit: f64,
    pub units: u64,
}

impl Sample {
    /// Build a sample from a bracketed counter delta.
    ///
    /// `before` / `after` are `snapshot()` reads taken immediately
    /// around the measured region; `units` is the number of rows (or
    /// calls) processed between them. The per-unit figures are exact
    /// rationals of integer counts — no floating-point measurement
    /// noise enters, which is what makes the downstream gate
    /// deterministic.
    #[must_use]
    pub fn from_delta(id: &str, before: (u64, u64), after: (u64, u64), units: u64) -> Self {
        let count_delta = after.0.saturating_sub(before.0);
        let bytes_delta = after.1.saturating_sub(before.1);
        let units_f = units.max(1) as f64;
        Self {
            id: id.to_string(),
            allocs_per_unit: count_delta as f64 / units_f,
            bytes_per_unit: bytes_delta as f64 / units_f,
            units,
        }
    }
}

/// Escape a string for embedding in a JSON value. The ids we emit are
/// ASCII bench names, so only the quote and backslash need handling;
/// kept explicit so the writer carries no serialization dependency
/// (benches must stay lean and the metric file shape is trivial).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// Write a metric file the gate script consumes.
///
/// Shape mirrors the committed baseline (`perf-gate/*.json`): a top
/// object keyed by sample id, each value carrying `allocs_per_unit`,
/// `bytes_per_unit`, and `units`. A leading `_meta` object records what
/// the file measures so a reader needs no external doc. The directory is
/// created if absent.
///
/// # Panics
///
/// Panics if the parent directory cannot be created or the file cannot
/// be written — a bench that cannot emit its metric must fail loudly so
/// the gate never silently reads a stale file.
pub fn write_metric_file(path: &Path, meta: &str, samples: &[Sample]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create perf-gate metric dir {}: {e}", parent.display()));
    }
    let mut body = String::new();
    body.push_str("{\n");
    body.push_str(&format!("  \"_meta\": \"{}\",\n", json_escape(meta)));
    for (idx, sample) in samples.iter().enumerate() {
        let comma = if idx + 1 < samples.len() { "," } else { "" };
        body.push_str(&format!(
            "  \"{id}\": {{ \"allocs_per_unit\": {allocs:.4}, \
             \"bytes_per_unit\": {bytes:.2}, \"units\": {units} }}{comma}\n",
            id = json_escape(&sample.id),
            allocs = sample.allocs_per_unit,
            bytes = sample.bytes_per_unit,
            units = sample.units,
        ));
    }
    body.push_str("}\n");
    std::fs::write(path, body)
        .unwrap_or_else(|e| panic!("write perf-gate metric file {}: {e}", path.display()));
    eprintln!(
        "perf-gate: wrote {} ({} sample(s))",
        path.display(),
        samples.len()
    );
}
