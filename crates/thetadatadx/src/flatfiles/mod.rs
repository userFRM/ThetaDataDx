//! FLATFILES surface: whole-universe daily snapshots over the legacy MDDS port.
//!
//! Pulls one INDEX + DATA blob per `(SecType, ReqType, date)` tuple from
//! `nj-{a,b}.thetadata.us:12000-12001` over a TLS PacketStream protocol
//! distinct from MDDS gRPC and FPSS streaming. Decodes the blob into one
//! row per `(contract, timestamp)` pair and emits CSV, JSONL, or a typed
//! in-memory `Vec<FlatFileRow>`.
//!
//! # Public entry points
//!
//! - [`flatfile_request`] — pull, decode, write CSV or JSONL to disk.
//! - [`flatfile_request_decoded`] — pull, decode, return `Vec<FlatFileRow>`
//!   in memory.
//! - [`flatfile_request_raw`] — pull only; write the raw INDEX + DATA blob
//!   to disk for callers that want to run their own decoder.
//!
//! All three are also reachable via the [`crate::ThetaDataDx`] client.
//!
//! # Wire protocol
//!
//! Every frame, in either direction, is encoded as
//! `[u32 size BE][u16 msg_code BE][i64 id BE][payload]`. `size` covers
//! `payload` only. `id` is `-1` for connection-scoped frames (CREDENTIALS,
//! VERSION, SESSION_TOKEN, METADATA) and a positive integer for
//! request/response correlation (FLAT_FILE = 217, FLAT_FILE_END = 218).
//!
//! Authentication is a CREDENTIALS frame (msg=0) followed by a VERSION
//! frame (msg=5). On success the server emits SESSION_TOKEN (msg=1) plus
//! METADATA (msg=3) carrying the account's bundle string. On failure it
//! emits DISCONNECTED (msg=102) with a 2-byte big-endian `RemoveReason`.
//!
//! A FLAT_FILE request frame (msg=217) carries an ASCII query payload
//! `MSG_CODE=217&id=<id>&start_date=YYYYMMDD&REQ=<reqcode>&SEC=<sectype>
//! &rth=true&ivl=0`. The server replies with a sequence of msg=217 frames
//! sharing the request id, each carrying a chunk of the INDEX + DATA
//! blob, terminated by a single FLAT_FILE_END (msg=218).
//!
//! # Server identity
//!
//! TLS connections are pinned to the ThetaData production keypair via
//! `mdds_spki::MddsSpkiVerifier` — the same SPKI hash that signs the FPSS
//! endpoints. The vendor's bundled `client.jks` truststore is not used.
//!
//! # Submodule layout
//!
//! - `framing` — PacketStream frame reader/writer.
//! - `mdds_spki` — TLS verifier with SNI allowlist + SPKI pin.
//! - `session` — TCP/TLS handshake plus CREDENTIALS + VERSION login.
//! - `request` — FLAT_FILE request driver and chunked response sink.
//! - `index` — header parser plus INDEX-section iterator.
//! - `decode` — FIT per-contract block decoder.
//! - `decoded` — end-to-end pull + decode + write driver.
//! - `decoded_row` — typed in-memory row representation.
//! - `writer` — CSV and JSONL row sinks.
//! - `format` — output format enum.
//! - `types` — `SecType`, `ReqType`, `FlatFilesUnavailableReason`.
//! - `datatype` — wire-code mapping for the per-row column schema.

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

pub use decoded::{default_output_filename, flatfile_request, flatfile_request_decoded};
pub use decoded_row::{FlatFileRow, FlatFileValue};
pub use format::FlatFileFormat;
pub use request::flatfile_request_raw;
pub use types::{FlatFilesUnavailableReason, ReqType, SecType};

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
