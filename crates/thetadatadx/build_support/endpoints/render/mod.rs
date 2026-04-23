//! Per-language emitters driven by the joined endpoint model.
//!
//! Split one-file-per-target so each emitter can stay focused on its language
//! while sharing `model.rs`, `helpers.rs`, and `modes.rs` upstream. The
//! `build_out` submodule owns the `OUT_DIR` artifacts consumed at
//! `include!()` time by the main crate; `sdk_files` orchestrates the
//! checked-in checked-in projections for the binary SDKs.

mod build_out;
mod cli_validate;
mod cpp;
mod cpp_validate;
mod enums;
mod ffi;
mod go;
mod go_validate;
mod mdds;
mod python;
mod python_validate;
mod sdk_files;
mod typescript;

pub use build_out::generate_all;
pub use sdk_files::{check_sdk_generated_files, write_sdk_generated_files};
