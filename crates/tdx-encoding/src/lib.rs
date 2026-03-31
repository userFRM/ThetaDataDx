//! # tdx-encoding -- Wire-format encoding for ThetaData FPSS protocol
//!
//! Pure encoding crate with **zero networking dependencies**. Contains:
//!
//! - **FIT/FIE codecs** -- 4-bit nibble encoding used by FPSS for tick compression
//! - **Type definitions** -- enums, fixed-point `Price`, tick structs
//! - **FPSS protocol** -- contract serialization, message builders, frame reader/writer
//!
//! This crate is the `dbn` to `thetadatadx`'s `databento-rs` -- it defines the wire
//! format without any client logic, authentication, or networking.
//!
//! ## Usage
//!
//! Most users should depend on `thetadatadx` (which re-exports everything from this
//! crate). Use `tdx-encoding` directly only if you need the wire format without the
//! full SDK (e.g., for offline replay, WASM, or embedded use).

pub mod codec;
pub mod error;
pub mod protocol;
pub mod types;

pub use error::Error;
