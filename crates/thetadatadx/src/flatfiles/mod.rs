//! FLATFILES — third public surface alongside FPSS and MDDS.
//!
//! # Identity
//!
//! FLATFILES is ThetaData's overnight-batch delivery channel for whole-universe
//! daily snapshots of options/stocks data. Every night the vendor pre-builds
//! one INDEX + DATA blob per `(sec_type, data_type, date)` tuple; customers can
//! pull the whole-universe blob in a single request and walk it locally to
//! produce CSV. The legacy Java terminal exposes a REST front-door
//! (`GET /v2/file/{secType}/{reqType}?start_date=YYYYMMDD`) that internally
//! talks to the vendor's MDDS legacy port (12000-12001 on `nj-{a,b}.thetadata.us`)
//! over a custom TLS PacketStream protocol.
//!
//! This Rust module is a **clean-room re-implementation** of the wire
//! protocol — not a port of decompiled Java source. Every structural fact
//! below was derived from observing live wire bytes against the production
//! server, plus high-level shape information from the bundled bytecode.
//!
//! # Module status
//!
//! | Layer                        | Status                              |
//! |------------------------------|-------------------------------------|
//! | TCP + TLS handshake          | working (SPKI pinned via `mdds_spki`) |
//! | PacketStream framing         | working (`framing` submodule)       |
//! | CREDENTIALS + VERSION login  | working (verified live 2026-04-29)  |
//! | FLAT_FILE request            | working (verified live 2026-04-29)  |
//! | Chunked response accumulator | working — writes raw stream to disk |
//! | INDEX walker                 | working (`index` submodule)         |
//! | FIT per-contract decoder     | working (`decode` submodule)        |
//! | CSV writer (vendor-format)   | working (`writer::CsvSink`)         |
//! | Parquet writer (zstd, Arrow) | working (`writer::ParquetSink`)     |
//! | JSONL writer                 | working (`writer::JsonlSink`)       |
//!
//! The low-level raw-stream path is exercised end-to-end by
//! [`flatfile_request_raw`], which authenticates, sends a single FLAT_FILE
//! request, accumulates every response chunk to a local scratch file, and
//! returns its path on `FLAT_FILE_END`. The decoded pipeline is also
//! implemented:
//! [`ThetaDataDx::flatfile_request`](crate::ThetaDataDx::flatfile_request)
//! walks the INDEX, decodes FIT records, and writes the requested typed
//! output format (CSV / Parquet / JSONL). The raw capture helper remains
//! available for debugging, fixtures, and byte-level verification.
//!
//! # PacketStream framing (verified live)
//!
//! Every message in either direction is `[u32 size BE][u16 msg_code BE]
//! [i64 id BE][payload]`. `size` is the byte length of `payload` only. `id`
//! is `-1` for connection-scoped frames (CREDENTIALS, VERSION, SESSION_TOKEN,
//! METADATA) and a per-call positive integer for request/response correlation
//! (FLAT_FILE / FLAT_FILE_END).
//!
//! # Auth (verified live)
//!
//! After the TLS handshake the client immediately writes:
//!
//! 1. `CREDENTIALS` (msg=0, id=-1) — payload is `[u8 0x00][u16 BE userlen]
//!    [user_utf8][pass_utf8]`. The leading byte is `0x00`. (Earlier
//!    reverse-engineering notes guessed `0x03`; the bytecode shows `iconst_0`
//!    and the live server rejects `0x03` with `INVALID_CREDENTIALS`.)
//! 2. `VERSION` (msg=5, id=-1) — payload is `[u32 BE jsonlen][json_utf8]`,
//!    where `json` is a flat map of system properties; the only required
//!    key the server inspects is `terminal.version`.
//!
//! On success the server pushes back, in order:
//! - `SESSION_TOKEN` (msg=1, id=-1) — 128 bytes of opaque token (kept by the
//!   server; the client doesn't need to echo it back during the same
//!   connection).
//! - `METADATA` (msg=3, id=-1) — ASCII bundle string, e.g.
//!   `"STOCK.STANDARD, OPTION.PRO, INDEX.FREE"`.
//!
//! On failure the server returns `DISCONNECTED` (msg=102) with a 2-byte
//! big-endian `RemoveReason` ordinal and closes the socket.
//!
//! # FLAT_FILE request (verified live)
//!
//! After auth, the client sends one frame: `msg=217`, `id=<arbitrary
//! positive i64>`, payload = ASCII query string `MSG_CODE=217&id=<id>&
//! start_date=YYYYMMDD&REQ=<reqcode>&SEC=<sectype>&rth=true&ivl=0`. The
//! `MSG_CODE` field is redundant with the framing message code; the server
//! checks both. `REQ` is the V2 ReqType wire code (e.g. `103` =
//! `OPEN_INTEREST`, `207` = `TRADE_QUOTE`, `1` = `EOD`); see
//! [`ReqType`] for the full mapping.
//!
//! # FLAT_FILE response (verified live)
//!
//! The server streams a sequence of `msg=217` frames sharing the same `id`
//! as the request. Each frame's payload is a chunk of the wire-format INDEX +
//! DATA blob; chunks are appended in order. A single `msg=218`
//! (FLAT_FILE_END) frame on the same id signals "all chunks delivered."
//! Live observation against `nj-a.thetadata.us:12000` for
//! `option_open_interest 20260428`: 8 chunks observed in the first 15 s,
//! each ~512 KB, headed by a small `[u32 u32 u32 u32]` tuple in the first
//! chunk that introduces the INDEX section.
//!
//! # Anti-IP discipline
//!
//! No constants, decode tables, or implementation details that constitute
//! ThetaData IP are reproduced here — only the public-facing wire shape
//! observable to any TLS-MITM. The `client.jks` truststore bundled in the
//! vendor jar is **not** used: server identity is established via SPKI
//! pinning to the production keypair (which also signs the FPSS endpoints).

pub(crate) mod datatype;
pub(crate) mod decode;
pub(crate) mod decoded;
pub(crate) mod format;
pub(crate) mod framing;
pub(crate) mod index;
pub(crate) mod mdds_spki;
pub(crate) mod request;
pub(crate) mod session;
pub(crate) mod types;
pub(crate) mod writer;

pub use decoded::{default_output_filename, flatfile_request};
pub use format::FlatFileFormat;
pub use request::flatfile_request_raw;
pub use types::{FlatFilesUnavailableReason, ReqType, SecType};

/// Decode an already-saved raw FLATFILES blob into a typed output file.
///
/// Test-facing helper used by the byte-match integration suite to share
/// one live capture across CSV / Parquet / JSONL smoke tests without
/// hitting the wire three times. Hidden from `docs.rs`; not part of the
/// stable public API.
#[doc(hidden)]
pub fn decoded_decode_to_file_for_test(
    raw_path: &std::path::Path,
    sec: SecType,
    output_path: &std::path::Path,
    format: FlatFileFormat,
) -> Result<(), crate::error::Error> {
    decoded::decode_to_file(raw_path, sec, output_path, format)
}
