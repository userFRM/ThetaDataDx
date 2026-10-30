//! Wire data types for the FPSS data layer: tick structs, protocol enums, and
//! the fixed-point [`price::Price`].
//!
//! Splits into three leaves — [`enums`] (wire enum taxonomy), [`price`]
//! (variable-precision fixed-point price), and [`tick`] (per-tick
//! `#[repr(C)]` structs). Callers reach the types through the leaf paths.

pub mod enums;
pub mod price;
pub mod tick;
