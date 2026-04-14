//! Canonical parser for the option `right` parameter.
//!
//! The implementation now lives in [`tdbe::right`] so that `tdbe::greeks`
//! can reuse it without `tdbe` reverse-depending on `thetadatadx`. This
//! module is a thin re-export layer kept for back-compat: callers using
//! `thetadatadx::right::parse_right` and friends continue to compile.
//!
//! See [`tdbe::right`] for the accepted vocabulary and full documentation.

pub use tdbe::right::{parse_right, parse_right_strict, ParsedRight};
