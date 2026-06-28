//! Per-language emitters for the checked-in SDK projections.
//!
//! Each submodule owns one render target (Python, TypeScript, C++, FFI,
//! per-language live-validators, enums). The build script never compiles
//! this tree — only the `generate_sdk_surfaces` binary reaches here.

mod config_accessors;
mod cpp;
mod cpp_validate;
mod doc;
mod enums;
mod ffi;
mod python;
mod python_stub;
mod python_validate;
mod sdk_files;
mod typescript;

pub(super) use sdk_files::{check_sdk_generated_files, write_sdk_generated_files};
