//! Order-preserving response rows.
//!
//! `sonic_rs::Value` objects do not preserve field insertion order on output
//! (their storage is an indexed key array whose iteration order is not
//! construction order), so the v3 CSV column sequence cannot be read back off a
//! serialized JSON row. [`Row`] fixes that: it is an ordered `(key, value)`
//! sequence that records declaration order, so the serializer's field order is
//! the single source of both the JSON body and the CSV header — the column
//! order lives in exactly one place (the serializer's [`row!`] block) rather
//! than being duplicated in a side table that drifts.
//!
//! Build a `Row` with the [`row!`] macro, which mirrors `sonic_rs::json!`
//! ergonomics:
//!
//! ```ignore
//! let r = row! {
//!     "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
//!     "open": t.open,
//!     "close": t.close,
//! };
//! ```
//!
//! A `Row` converts to a `sonic_rs` object for the JSON envelope
//! ([`Row::into_value`]) and yields its columns in declaration order for the CSV
//! header ([`Row::columns`]).

use sonic_rs::prelude::*;

/// An order-preserving response row: `(key, value)` pairs in declaration order.
///
/// The key is `&'static str` (every response field name is a string literal in
/// a serializer's [`row!`] block), so a `Row` borrows its keys for free and the
/// CSV header can hand them out without allocation.
#[derive(Clone, Default)]
pub struct Row(Vec<(&'static str, sonic_rs::Value)>);

impl Row {
    /// An empty row with capacity for `cap` fields (avoids reallocation while a
    /// serializer pushes a known column count).
    pub fn with_capacity(cap: usize) -> Self {
        Row(Vec::with_capacity(cap))
    }

    /// Append a field, preserving declaration order. A repeated key is appended
    /// again rather than overwriting — serializers never emit a key twice, and
    /// preserving every push keeps the type a faithful ordered record.
    pub fn push(&mut self, key: &'static str, value: impl Into<sonic_rs::Value>) {
        self.0.push((key, value.into()));
    }

    /// Append a pre-built `sonic_rs::Value`. The [`row!`] macro routes every
    /// field value through `sonic_rs::json!`, so it lands here with the exact
    /// same primitive handling the old object-literal path used (notably the
    /// non-finite-`f64`-to-null collapse, which `From<f64>` cannot express).
    pub fn push_value(&mut self, key: &'static str, value: sonic_rs::Value) {
        self.0.push((key, value));
    }

    /// Insert a field at `index`, shifting later fields right. Used to splice
    /// the contract-identity columns into their v3 slot (leading, or just after
    /// the `timestamp` column) without disturbing the rest of the order.
    pub fn insert(&mut self, index: usize, key: &'static str, value: impl Into<sonic_rs::Value>) {
        self.0.insert(index, (key, value.into()));
    }

    /// Retain only the fields whose key satisfies `keep`, preserving order.
    /// Used to lift the contract-identity columns out of their temporary
    /// trailing position before re-inserting them at the v3 slot.
    pub fn retain(&mut self, mut keep: impl FnMut(&'static str) -> bool) {
        self.0.retain(|&(k, _)| keep(k));
    }

    /// The value for `key`, or `None` if the row does not carry it. The first
    /// match wins (serializers never emit a key twice).
    pub fn get(&self, key: &str) -> Option<&sonic_rs::Value> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// The number of fields in the row. The splice path reads this to clamp the
    /// `AfterTimestamp` insertion index.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The column keys in declaration order (the CSV header sequence).
    pub fn columns(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().map(|(k, _)| *k)
    }

    /// Convert to a `sonic_rs` object for the JSON envelope.
    ///
    /// The JSON object's on-wire field order is not a stable contract (the
    /// vendor serialises every body from an unordered map and clients read by
    /// key), so the order is not preserved here — only the columns + values
    /// are. The CSV path, which *is* positional, reads order from [`Row`]
    /// directly via [`Row::columns`].
    pub fn into_value(self) -> sonic_rs::Value {
        let mut out = sonic_rs::json!({});
        let object = out
            .as_object_mut()
            .expect("freshly built JSON object is an object");
        for (key, value) in self.0 {
            object.insert(key, value);
        }
        out
    }
}

/// Build an order-preserving [`Row`] from `"key": expr` pairs.
///
/// Mirrors `sonic_rs::json!` ergonomics for object literals but yields a [`Row`]
/// that records declaration order, so the serializer's field order drives both
/// the JSON body and the CSV header. A trailing comma is allowed.
///
/// ```ignore
/// let r = row! {
///     "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
///     "open": t.open,
/// };
/// ```
#[macro_export]
macro_rules! row {
    ( $( $key:literal : $value:expr ),* $(,)? ) => {{
        // Count the fields so the backing `Vec` is sized once. Each key maps to
        // a `()` so the array length is the field count, evaluated at compile
        // time.
        let mut row = $crate::row::Row::with_capacity(
            <[()]>::len(&[ $( $crate::row::row!(@unit $key) ),* ])
        );
        // Route every value through `sonic_rs::json!` so primitives convert
        // exactly as the old object-literal path did — in particular the
        // non-finite-`f64`-to-null collapse, which a plain `From<f64>` cannot
        // perform (`f64` has only `TryFrom<Value>`).
        $( row.push_value($key, ::sonic_rs::json!($value)); )*
        row
    }};
    // Internal: discard a key literal, yielding a unit for the count array.
    (@unit $key:literal) => { () };
}

// Re-export so `crate::row::row!` resolves (the `#[macro_export]` form lands at
// the crate root, but the serializers refer to it through the module path).
pub use crate::row;
