//! FLATFILES: whole-universe daily snapshots over the legacy MDDS
//! port. Pulls one INDEX + DATA blob per `(SecType, ReqType, date)`
//! tuple and emits CSV, JSONL, or `Vec<FlatFileRow>`.
//!
//! Public entry points: [`flatfile_request`] (write to disk),
//! [`flatfile_request_decoded`] (in-memory `Vec<FlatFileRow>`),
//! [`flatfile_request_raw`] (raw INDEX + DATA blob). All three are
//! also reachable via [`crate::Client`].
//!
//! Server identity is SPKI-pinned via the internal
//! `mdds_spki::MddsSpkiVerifier`. On-disk blob layout is documented
//! at the module level in `crate::flatfiles::index` (private; see
//! `cargo doc --document-private-items`).

pub(crate) mod datatype;
pub(crate) mod decode;
pub(crate) mod decoded;
pub(crate) mod decoded_row;
pub(crate) mod format;
pub(crate) mod framing;
pub(crate) mod index;
pub(crate) mod mdds_spki;
pub(crate) mod request;
pub(crate) mod session;
pub(crate) mod types;
pub(crate) mod writer;

/// Dynamic-schema Arrow conversion for [`FlatFileRow`] collections.
///
/// Gated behind the `arrow` feature. Exposed as `pub` (not `pub(crate)`) so
/// the language bindings (Python, TypeScript, C++ FFI) can route their
/// `to_arrow` terminals through one SSOT implementation rather than
/// re-deriving the schema on each surface.
#[cfg(feature = "arrow")]
pub mod arrow;

pub use decoded::{
    flatfile_request, flatfile_request_decoded, flatfile_request_decoded_with_config,
    flatfile_request_with_config,
};

/// Removes `path` on drop unless [`disarm`](Self::disarm) has been called.
///
/// Reaps a scratch / temp file on every exit path — a `?` early return, an
/// error, or a dropped (cancelled) future — where a single post-await cleanup
/// line would be skipped on cancellation. `disarm` hands ownership back to the
/// caller once the file has been renamed onto its final name or returned on
/// success. Cleanup is best-effort and ignores a missing file.
// ponytail: sync unlink in Drop; a single best-effort syscall, no async needed.
pub(crate) struct ScratchGuard {
    path: std::path::PathBuf,
    armed: bool,
}

impl ScratchGuard {
    pub(crate) fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            armed: true,
        }
    }

    /// Cancel removal — the file is now the caller's (renamed onto its final
    /// name or handed back on success).
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
pub use decoded_row::{FlatFileRow, FlatFileValue};
pub use format::FlatFileFormat;
pub use request::{flatfile_request_raw, flatfile_request_raw_with_config};
pub use types::{flat_file_serves, FlatFilesUnavailableReason, ReqType, SecType, SERVED_DATASETS};

/// Decode an already-saved raw FLATFILES blob into a typed output file.
///
/// Test-facing helper used by the byte-match integration suite to share
/// one live capture across CSV / JSONL smoke tests without hitting the
/// wire twice. Hidden from `docs.rs`; not part of the stable public API.
#[doc(hidden)]
pub fn decoded_decode_to_file_for_test(
    raw_path: &std::path::Path,
    sec: SecType,
    output_path: &std::path::Path,
    format: FlatFileFormat,
) -> Result<(), crate::error::Error> {
    decoded::decode_to_file(raw_path, sec, output_path, format)
}
