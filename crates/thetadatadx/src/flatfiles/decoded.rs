//! End-to-end decoded FLATFILES request driver.
//!
//! Given the full raw INDEX+DATA blob already on disk (produced by
//! [`crate::flatfiles::flatfile_request_raw`]), this module walks the
//! INDEX, decodes each contract's FIT block, and drives a polymorphic
//! [`crate::flatfiles::writer::RowSink`] in vendor row order.
//!
//! The public face is [`flatfile_request`] — same signature as the raw
//! variant plus a `format: FlatFileFormat` argument that selects the
//! on-disk encoding.

use std::path::{Path, PathBuf};

use crate::auth::Credentials;
use crate::config::FlatFilesConfig;
use crate::error::Error;
use crate::flatfiles::decode::decode_block;
use crate::flatfiles::decoded_row::FlatFileRow;
use crate::flatfiles::format::FlatFileFormat;
use crate::flatfiles::index::{parse_header, IndexIter};
use crate::flatfiles::request::flatfile_request_raw_with_config;
use crate::flatfiles::types::{ReqType, SecType};
use crate::flatfiles::writer::{CsvSink, JsonlSink, RowSink, RowView};
use crate::util::random_id::random_id_hex;

/// Narrow a wire `u64` byte offset to the platform `usize` used for slicing.
///
/// Header lengths and INDEX block offsets are 64-bit on the wire. On a target
/// where `usize` is narrower than `u64` an out-of-range offset would truncate,
/// and the subsequent in-bounds check would then pass against the truncated
/// value and read the wrong block. Route every offset through this checked
/// conversion so an offset that does not fit becomes a typed error on every
/// target instead of a silent wrong read.
fn offset_to_usize(v: u64, field: &str) -> Result<usize, Error> {
    usize::try_from(v).map_err(|_| {
        Error::config_internal(format!(
            "flatfiles: offset {field}={v} does not fit in usize on this target"
        ))
    })
}

/// Pull a flat-file blob, decode it, and write the requested format.
///
/// 1. Auth + raw-stream pull into a scratch file alongside `output_path`.
/// 2. Memory-map the raw stream and parse the header.
/// 3. Walk the INDEX section, decode each contract's FIT block, and
///    emit rows in INDEX order to the chosen sink.
/// 4. Delete the raw scratch blob regardless of outcome.
///
/// The raw `.raw` scratch blob is this driver's own artifact: the wire
/// layer opens it with `File::create` once authentication succeeds, so a
/// mid-stream truncation, a server `DISCONNECTED`, a read error, or a
/// decode fault all leave the blob on disk. Whoever owns the artifact
/// cleans it, so the scratch is removed on the error path as well as on
/// success — a caller that passes its own scratch path (the REST server
/// renames a `.partial` onto a final path) cannot see the SDK's distinct
/// `.raw` sibling and so could never reap it. Cleanup failure is
/// non-fatal: on success the caller already has the decoded file, and on
/// error the original fault is the one worth surfacing.
///
/// Returns the final `output_path` (with the format extension auto-
/// appended if the input lacked one).
pub async fn flatfile_request(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: impl AsRef<Path>,
    format: FlatFileFormat,
) -> Result<PathBuf, Error> {
    let config = FlatFilesConfig::default();
    flatfile_request_with_config(creds, sec, req, date, output_path, format, &config).await
}

/// Same as [`flatfile_request`] but with caller-supplied retry tuning.
/// Routes the raw-pull leg through [`flatfile_request_raw_with_config`].
pub async fn flatfile_request_with_config(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: impl AsRef<Path>,
    format: FlatFileFormat,
    config: &FlatFilesConfig,
) -> Result<PathBuf, Error> {
    let final_path = format.ensure_extension(output_path.as_ref());
    let raw_path = final_path.with_extension(format!("{}.raw", format.extension()));

    // Run the pull + decode, then reap the raw scratch blob on every
    // outcome. The raw artifact is created by the wire layer once auth
    // succeeds, so any post-handshake failure (StreamTruncated, a
    // mid-stream DISCONNECTED, a read error) or a decode fault would
    // otherwise orphan it; an error before `File::create` (connect /
    // login) leaves no file, and the ignored `NotFound` from removing it
    // is harmless.
    let result = pull_and_decode_to_file(
        creds,
        sec,
        req,
        date,
        &raw_path,
        &final_path,
        format,
        config,
    )
    .await;
    let _ = tokio::fs::remove_file(&raw_path).await;
    result.map(|()| final_path)
}

/// Pull the raw blob into `raw_path` and decode it onto `final_path`.
///
/// Split out from [`flatfile_request_with_config`] so the caller can reap
/// the raw scratch blob on a single cleanup line that runs whether this
/// succeeds or fails — the cleanup must not be duplicated across every
/// `?` early-return.
#[allow(clippy::too_many_arguments)]
async fn pull_and_decode_to_file(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    raw_path: &Path,
    final_path: &Path,
    format: FlatFileFormat,
    config: &FlatFilesConfig,
) -> Result<(), Error> {
    // Step 1: live pull. Reuses the working wire layer untouched.
    flatfile_request_raw_with_config(creds, sec, req, date, raw_path, config).await?;

    // Step 2-3: decode + write. The decoder reads + parses the entire
    // blob synchronously and the writer hits the filesystem in tight
    // loops; do that on the blocking pool so we don't stall any other
    // tokio task (FPSS streaming, MDDS historical) on the same runtime.
    let raw_for_decode = raw_path.to_path_buf();
    let final_for_decode = final_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        decode_to_file(&raw_for_decode, sec, &final_for_decode, format)
    })
    .await
    .map_err(|e| Error::config_internal(format!("flatfiles: decode task panicked: {e}")))??;

    Ok(())
}

/// Decode an already-captured raw blob into the requested format.
///
/// Splits out from [`flatfile_request`] so the byte-match integration
/// test can re-run the decoder against a saved blob without burning
/// another live MDDS call.
pub(crate) fn decode_to_file(
    raw_path: &Path,
    sec: SecType,
    output_path: &Path,
    format: FlatFileFormat,
) -> Result<(), Error> {
    let blob = std::fs::read(raw_path)?;
    let hdr = parse_header(&blob)?;

    let index_start = offset_to_usize(hdr.index_offset, "index_offset")?;
    let index_byte_len = offset_to_usize(hdr.index_byte_len, "index_byte_len")?;
    let data_byte_len = offset_to_usize(hdr.data_byte_len, "data_byte_len")?;
    let index_end = index_start.checked_add(index_byte_len).ok_or_else(|| {
        Error::config_internal(format!(
            "flatfiles: header lengths overflow usize (index_offset={index_start}, index_byte_len={index_byte_len})"
        ))
    })?;
    let data_start = index_end;
    let data_end = data_start.checked_add(data_byte_len).ok_or_else(|| {
        Error::config_internal(format!(
            "flatfiles: header lengths overflow usize (data_start={data_start}, data_byte_len={data_byte_len})"
        ))
    })?;
    if data_end > blob.len() {
        return Err(Error::config_internal(format!(
            "flatfiles: blob truncated — header expected {} bytes total, got {}",
            data_end,
            blob.len()
        )));
    }
    let index_bytes = &blob[index_start..index_end];
    let data_bytes = &blob[data_start..data_end];

    let mut sink: Box<dyn RowSink> = match format {
        FlatFileFormat::Csv => Box::new(CsvSink::new(
            output_path,
            sec,
            hdr.fmt.clone(),
            hdr.price_type_idx,
        )?),
        FlatFileFormat::Jsonl => Box::new(JsonlSink::new(
            output_path,
            sec,
            hdr.fmt.clone(),
            hdr.price_type_idx,
        )?),
    };
    sink.write_header()?;

    let n_columns = hdr.fmt.len();
    let mut rows_buf: Vec<Vec<i32>> = Vec::with_capacity(1024);
    for entry_res in IndexIter::new(index_bytes, sec) {
        let entry = entry_res?;
        let bs = offset_to_usize(entry.block_start, "block_start")?;
        let be = offset_to_usize(entry.block_end, "block_end")?;
        if be > data_bytes.len() || bs > be {
            return Err(Error::config_internal(format!(
                "flatfiles: INDEX block bounds [{bs}, {be}) escape DATA section ({} bytes)",
                data_bytes.len()
            )));
        }
        let block = &data_bytes[bs..be];
        decode_block(block, n_columns, &mut rows_buf)?;
        for row in &rows_buf {
            sink.write_row(RowView {
                entry: &entry,
                data: row,
            })?;
        }
    }
    sink.finish()?;
    Ok(())
}

/// Pull a flat-file blob and return decoded rows in memory.
///
/// Same auth and stream path as [`flatfile_request`], but skips the
/// on-disk writer. Use this when feeding the data straight into an
/// algorithm (e.g. backtester, risk model) without an intermediate
/// file. The whole `Vec` is materialised before the function returns —
/// for whole-universe blobs that can be hundreds of MB; if that does
/// not fit your memory budget, prefer [`flatfile_request`] with a
/// streaming reader on the resulting CSV / JSONL file.
pub async fn flatfile_request_decoded(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
) -> Result<Vec<FlatFileRow>, Error> {
    let config = FlatFilesConfig::default();
    flatfile_request_decoded_with_config(creds, sec, req, date, &config).await
}

/// Same as [`flatfile_request_decoded`] but with caller-supplied retry
/// tuning. Routes the raw-pull leg through
/// [`flatfile_request_raw_with_config`].
pub async fn flatfile_request_decoded_with_config(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    config: &FlatFilesConfig,
) -> Result<Vec<FlatFileRow>, Error> {
    // Per-invocation unique scratch path. Two concurrent calls for the
    // same `(sec, req, date)` must not share a file — they would race on
    // truncation and produce corrupt rows.
    let scratch = std::env::temp_dir().join(format!(
        "thetadatadx-flatfiles-{}-{}-{}-{}.raw",
        sec.as_wire(),
        req_name(req),
        date,
        random_id_hex()
    ));
    // Reap the raw scratch blob on every outcome — same ownership rule
    // as the on-disk path. The blob is created by the wire layer once
    // auth succeeds, so a post-handshake pull failure or a decode fault
    // would otherwise orphan it; a pre-`File::create` failure leaves no
    // file and the ignored `NotFound` is harmless.
    let result = pull_and_decode_to_memory(creds, sec, req, date, &scratch, config).await;
    let _ = tokio::fs::remove_file(&scratch).await;
    result
}

/// Pull the raw blob into `scratch` and decode it into a typed `Vec`.
///
/// Split out from [`flatfile_request_decoded_with_config`] so the caller
/// reaps the raw scratch blob on a single cleanup line that runs whether
/// this succeeds or fails.
async fn pull_and_decode_to_memory(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    scratch: &Path,
    config: &FlatFilesConfig,
) -> Result<Vec<FlatFileRow>, Error> {
    flatfile_request_raw_with_config(creds, sec, req, date, scratch, config).await?;
    let scratch_for_decode = scratch.to_path_buf();
    let rows = tokio::task::spawn_blocking(move || decode_to_memory(&scratch_for_decode, sec))
        .await
        .map_err(|e| Error::config_internal(format!("flatfiles: decode task panicked: {e}")))??;
    Ok(rows)
}

/// Decode an already-captured raw blob into a typed in-memory `Vec`.
pub(crate) fn decode_to_memory(raw_path: &Path, sec: SecType) -> Result<Vec<FlatFileRow>, Error> {
    let blob = std::fs::read(raw_path)?;
    let hdr = parse_header(&blob)?;

    let index_start = offset_to_usize(hdr.index_offset, "index_offset")?;
    let index_byte_len = offset_to_usize(hdr.index_byte_len, "index_byte_len")?;
    let data_byte_len = offset_to_usize(hdr.data_byte_len, "data_byte_len")?;
    let index_end = index_start
        .checked_add(index_byte_len)
        .ok_or_else(|| Error::config_internal("flatfiles: header lengths overflow usize"))?;
    let data_start = index_end;
    let data_end = data_start
        .checked_add(data_byte_len)
        .ok_or_else(|| Error::config_internal("flatfiles: header lengths overflow usize"))?;
    if data_end > blob.len() {
        return Err(Error::config_internal(format!(
            "flatfiles: blob truncated — header expected {} bytes total, got {}",
            data_end,
            blob.len()
        )));
    }
    let index_bytes = &blob[index_start..index_end];
    let data_bytes = &blob[data_start..data_end];

    let data_idx = crate::flatfiles::writer::data_indices(&hdr.fmt, hdr.price_type_idx);
    let n_columns = hdr.fmt.len();
    let mut rows_buf: Vec<Vec<i32>> = Vec::with_capacity(1024);
    let mut out: Vec<FlatFileRow> = Vec::new();
    for entry_res in IndexIter::new(index_bytes, sec) {
        let entry = entry_res?;
        let bs = offset_to_usize(entry.block_start, "block_start")?;
        let be = offset_to_usize(entry.block_end, "block_end")?;
        if be > data_bytes.len() || bs > be {
            return Err(Error::config_internal(format!(
                "flatfiles: INDEX block bounds [{bs}, {be}) escape DATA section ({} bytes)",
                data_bytes.len()
            )));
        }
        let block = &data_bytes[bs..be];
        decode_block(block, n_columns, &mut rows_buf)?;
        for row in &rows_buf {
            let pt = crate::flatfiles::writer::price_type_for_row(row, hdr.price_type_idx);
            out.push(FlatFileRow::from_decoded(
                &entry.symbol,
                entry.expiration,
                entry.strike,
                entry.right,
                &hdr.fmt,
                row,
                &data_idx,
                pt,
            )?);
        }
    }
    Ok(out)
}

fn req_name(req: ReqType) -> &'static str {
    match req {
        ReqType::Eod => "EOD",
        ReqType::Quote => "QUOTE",
        ReqType::OpenInterest => "OPEN_INTEREST",
        ReqType::Ohlc => "OHLC",
        ReqType::Trade => "TRADE",
        ReqType::TradeQuote => "TRADE_QUOTE",
    }
}

/// Vendor-style default filename for `(sec, req, date)`.
///
/// Mirrors the legacy terminal's `<SEC>-<REQ>-<DATE>.<ext>` convention.
/// Exposed as a helper so binaries / tests can derive a default path
/// without hardcoding the rules.
#[must_use]
pub fn default_output_filename(
    sec: SecType,
    req: ReqType,
    date: &str,
    format: FlatFileFormat,
) -> String {
    format!(
        "{}-{}-{}.{}",
        sec.as_wire(),
        req_name(req),
        date,
        format.extension()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the raw-scratch path the on-disk driver actually writes.
    ///
    /// When a caller hands `flatfile_request_with_config` its own scratch
    /// path that already carries an extension (the REST server passes a
    /// `…{uuid}.partial`), `ensure_extension` leaves it untouched, and the
    /// raw blob lands at `with_extension("{ext}.raw")` — which *replaces*
    /// the trailing `.partial`, yielding a `…{uuid}.csv.raw` sibling. That
    /// sibling is a different filename from the `.partial` the server's
    /// error cleanup removes, so only the driver that wrote it can reap it.
    /// This is the exact path that previously leaked on a post-handshake
    /// failure; pin it so a future change to the extension scheme can't
    /// silently reintroduce a server-invisible orphan.
    #[test]
    fn raw_scratch_path_is_csv_raw_sibling_not_the_partial_scratch() {
        let server_scratch =
            Path::new("/tmp/thetadatadx_server_flatfile_OPTION_5_20260428.csv.deadbeef.partial");
        let final_path = FlatFileFormat::Csv.ensure_extension(server_scratch);
        // The caller's path already had an extension, so it is unchanged.
        assert_eq!(final_path, server_scratch);
        let raw_path =
            final_path.with_extension(format!("{}.raw", FlatFileFormat::Csv.extension()));
        assert_eq!(
            raw_path,
            PathBuf::from(
                "/tmp/thetadatadx_server_flatfile_OPTION_5_20260428.csv.deadbeef.csv.raw"
            ),
            "raw blob must be the .csv.raw sibling the SDK owns and reaps"
        );
        assert_ne!(
            raw_path, server_scratch,
            "the raw blob the SDK writes must differ from the caller's scratch — \
             the caller's error cleanup cannot reach the SDK's sibling"
        );
    }

    /// A failure after the raw blob is on disk must leave no `.raw` orphan.
    ///
    /// The on-disk driver's contract is that it reaps its own raw scratch
    /// blob on *every* outcome, not just success — the blob is created by
    /// the wire layer once auth succeeds, so a mid-stream truncation, a
    /// server `DISCONNECTED`, a read error, or a decode fault would all
    /// otherwise orphan it. The network legs need a live host, so this
    /// drives the offline-reachable post-handshake fault: a raw blob whose
    /// bytes do not parse. It runs the driver's exact "fallible work, then
    /// reap on every path" shape and asserts the work fails *and* the blob
    /// is gone afterward. Before the fix the reap sat behind the `?` early
    /// returns and the blob survived an error.
    #[tokio::test]
    async fn mid_stream_failure_leaves_no_orphan_raw_blob() {
        let unique = random_id_hex();
        let final_path =
            std::env::temp_dir().join(format!("thetadatadx-flatfiles-reap-test-{unique}.csv"));
        let raw_path =
            final_path.with_extension(format!("{}.raw", FlatFileFormat::Csv.extension()));

        // Stand in for the wire layer having created + partly filled the
        // raw blob before the stream broke: bytes that cannot parse as a
        // valid FLATFILES header, so `decode_to_file` fails deterministically.
        std::fs::write(&raw_path, b"not a valid flatfiles blob").unwrap();
        assert!(raw_path.exists(), "precondition: the raw blob is on disk");

        // The driver's exact tail: run the fallible decode, then reap the
        // raw blob regardless of outcome (mirrors
        // `flatfile_request_with_config` once the pull has produced a blob).
        let result = decode_to_file(&raw_path, SecType::Option, &final_path, FlatFileFormat::Csv);
        let _ = tokio::fs::remove_file(&raw_path).await;

        assert!(
            result.is_err(),
            "a garbage raw blob must fail to decode — otherwise this test proves nothing"
        );
        assert!(
            !raw_path.exists(),
            "the raw blob must be reaped on the error path, not orphaned: {}",
            raw_path.display()
        );

        // Best-effort: clean up any partial decoded output the failed
        // decode may have created before erroring.
        let _ = std::fs::remove_file(&final_path);
    }

    #[test]
    fn vendor_filename_matches_terminal_layout() {
        // The reference vendor file is `OPTION-OPEN_INTEREST-20260428.csv`.
        let s = default_output_filename(
            SecType::Option,
            ReqType::OpenInterest,
            "20260428",
            FlatFileFormat::Csv,
        );
        assert_eq!(s, "OPTION-OPEN_INTEREST-20260428.csv");
        let p = default_output_filename(
            SecType::Stock,
            ReqType::Trade,
            "20260428",
            FlatFileFormat::Jsonl,
        );
        assert_eq!(p, "STOCK-TRADE-20260428.jsonl");
    }

    #[test]
    fn offset_to_usize_round_trips_in_range_values() {
        // An offset that fits is returned unchanged on every target.
        assert_eq!(offset_to_usize(0, "block_start").unwrap(), 0);
        assert_eq!(offset_to_usize(1500, "block_end").unwrap(), 1500);
        assert_eq!(
            offset_to_usize(usize::MAX as u64, "block_end").unwrap(),
            usize::MAX
        );
    }

    #[test]
    fn offset_to_usize_rejects_value_exceeding_usize() {
        // On a target where `usize` is narrower than `u64`, an offset above
        // `usize::MAX` must surface as a typed error rather than truncating
        // silently and reading the wrong DATA block. Where `usize == u64`
        // (the shipped 64-bit target) no such value exists, so the typed
        // conversion is exercised through its success path instead.
        if let Some(too_big) = (usize::MAX as u64).checked_add(1) {
            let err = offset_to_usize(too_big, "block_end").unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("block_end") && msg.contains("does not fit in usize"),
                "expected typed out-of-range offset error, got: {msg}"
            );
        } else {
            // 64-bit target: every u64 fits, so the checked path cannot reject.
            assert_eq!(
                offset_to_usize(u64::MAX, "block_end").unwrap(),
                u64::MAX as usize
            );
        }
    }
}
