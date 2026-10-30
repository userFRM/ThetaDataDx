//! Per-language emitters for the checked-in SDK projections.
//!
//! Each submodule owns one render target (the Python type stub, C++, the C
//! header, the live validator cells). The build script never compiles
//! this tree — only the `generate_sdk_surfaces` binary reaches here.

mod config_accessors;
mod cpp;
mod doc;
mod ffi;
mod python_stub;
mod sdk_files;
mod validator;

pub(super) use sdk_files::{check_sdk_generated_files, write_sdk_generated_files};
