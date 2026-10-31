//! Per-tick `#[repr(C, align(N))]` struct definitions, expanded from the
//! tick table (`crate::__ticks!`), plus the items the table does not express:
//!
//! * `impl TradeTick` flag helpers (`is_cancelled`, ...). These read
//!   `flags::*` constants.
//! * `impl OptionContract` for `is_call` / `is_put`: its contract columns are
//!   its own rather than the `#[contract_id]` triple.

/// Expands every table entry into its struct.
macro_rules! tick_structs {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        $(crate::__tick! { [$(#[$attr])* $name $body] => tick_struct {} })*
    };
}

/// One tick struct, its epoch accessors and, on a `#[contract_id]` row, the
/// contract helpers.
macro_rules! tick_struct {
    ($name:ident pod $($rest:tt)*) => {
        tick_struct! { @struct [#[derive(Debug, Clone, Copy)] #[repr(C, align(64))]] $name $($rest)* }
    };
    ($name:ident owned $($rest:tt)*) => {
        tick_struct! { @struct [#[derive(Debug, Clone)] #[repr(C)]] $name $($rest)* }
    };
    (@struct [$($attrs:tt)*] $name:ident contract [$($exp:ident $strike:ident $right:ident)?]
        doc [$head:tt [$($n:tt)?] [$($tail:tt)?]] summary $summary:tt more [$($more:tt)*]
        required $required:tt columns [$({ $f:ident $w:ident $ty:ident [$($has:ident)?] $d:tt })*]
        timestamps $timestamps:tt flags $flags:tt) => {
        #[doc = concat!($head $(, " -- ", $n, " fields.")? $(, " ", $tail)?)]
        $(#[doc = ""] #[doc = $more])*
        #[must_use]
        $($attrs)*
        pub struct $name {
            $(
                #[doc = $d]
                pub $f: tick_struct!(@type $ty),
                $(
                    #[doc = concat!("Whether the response carried a value for `", stringify!($f), "`. False means the")]
                    #[doc = "vendor sent no value, which is not the same as the zero this"]
                    #[doc = "field then holds: zero is a code the vendor assigns a meaning."]
                    pub $has: bool,
                )?
            )*
            $(
                /// Contract expiration (`YYYYMMDD`), or 0 when the response does not carry it.
                pub $exp: i32,
                /// Contract strike price in dollars, or 0.0 when the response does not carry it.
                pub $strike: f64,
                /// Contract right: `'C'` for a call, `'P'` for a put, or `'\0'` when the response does not carry it.
                pub $right: char,
            )?
        }

        tick_struct! { @count [$($n)?] [$($f)* $($exp $strike $right)?] }
        tick_struct! { @timestamps $name $timestamps }

        $(
            impl $name {
                /// `true` when `right` is `'C'` (call).
                #[inline]
                pub fn is_call(&self) -> bool {
                    self.$right == 'C'
                }
                /// `true` when `right` is `'P'` (put).
                #[inline]
                pub fn is_put(&self) -> bool {
                    self.$right == 'P'
                }
                /// `true` when the server populated contract identification fields.
                #[inline]
                pub fn has_contract_id(&self) -> bool {
                    self.$exp != 0
                }
            }
        )?
    };
    // The documented field count is the columns plus the contract triple.
    (@count [] $fields:tt) => {};
    (@count [$n:tt] [$($f:ident)*]) => {
        const _: () = assert!($n == [$(stringify!($f)),*].len());
    };
    (@timestamps $name:ident []) => {};
    (@timestamps $name:ident [$({ $acc:ident $date:ident $ms:ident })+]) => {
        impl $name {
            $(
                #[doc = concat!("Unix epoch milliseconds (UTC, DST-aware) combining `", stringify!($date), "`")]
                #[doc = concat!("with `", stringify!($ms), "` (Eastern-Time milliseconds-of-day). Returns")]
                #[doc = concat!("`None` when `", stringify!($date), "` is absent (`0`) or `", stringify!($ms), "` is outside")]
                #[doc = "the milliseconds-of-day domain. The raw integer fields stay"]
                #[doc = "primary; this accessor is a convenience at the epoch boundary."]
                #[must_use]
                pub fn $acc(&self) -> Option<i64> {
                    crate::tdbe::time::date_ms_to_epoch_ms(self.$date, self.$ms)
                }
            )+
        }
    };
    (@type i32) => { i32 };
    (@type i64) => { i64 };
    (@type f64) => { f64 };
    (@type price) => { f64 };
    (@type String) => { String };
    (@type right) => { char };
    (@type calendar_status) => { crate::tdbe::types::enums::CalendarStatus };
    (@type eod_num) => { i32 };
    (@type eod_num64) => { i64 };
    (@type eod_date) => { i32 };
    (@type eod_price) => { f64 };
}

crate::__ticks! { tick_structs {} }

// ─────────────────────────────────────────────────────────────────────────────
//  Hand-written impl blocks
// ─────────────────────────────────────────────────────────────────────────────

use crate::tdbe::flags;

impl TradeTick {
    /// `true` when the trade condition falls in the cancellation range.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        flags::trade::CANCELLED_RANGE.contains(&self.condition)
    }
}

impl OptionContract {
    /// `true` when `right` is `'C'` (call).
    #[inline]
    pub fn is_call(&self) -> bool {
        self.right == 'C'
    }
    /// `true` when `right` is `'P'` (put).
    #[inline]
    pub fn is_put(&self) -> bool {
        self.right == 'P'
    }
}
