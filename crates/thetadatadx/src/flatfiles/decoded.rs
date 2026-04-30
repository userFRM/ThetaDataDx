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
use crate::error::Error;
use crate::flatfiles::decode::decode_block;
use crate::flatfiles::decoded_row::FlatFileRow;
use crate::flatfiles::format::FlatFileFormat;
use crate::flatfiles::index::{parse_header, IndexIter};
use crate::flatfiles::request::flatfile_request_raw;
use crate::flatfiles::types::{ReqType, SecType};
use crate::flatfiles::writer::{CsvSink, JsonlSink, RowSink, RowView};

/// Pull a flat-file blob, decode it, and write the requested format.
///
/// 1. Auth + raw-stream pull into a scratch file alongside `output_path`.
/// 2. Memory-map the raw stream and parse the header.
/// 3. Walk the INDEX section, decode each contract's FIT block, and
///    emit rows in INDEX order to the chosen sink.
/// 4. Delete the scratch file on success.
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
    let final_path = format.ensure_extension(output_path.as_ref());
    let raw_path = final_path.with_extension(format!("{}.raw", format.extension()));

    // Step 1: live pull. Reuses the working wire layer untouched.
    flatfile_request_raw(creds, sec, req, date, &raw_path).await?;

    // Step 2-3: decode + write. The decoder reads + parses the entire
    // blob synchronously and the writer hits the filesystem in tight
    // loops; do that on the blocking pool so we don't stall any other
    // tokio task (FPSS streaming, MDDS gRPC) on the same runtime.
    let raw_for_decode = raw_path.clone();
    let final_for_decode = final_path.clone();
    tokio::task::spawn_blocking(move || {
        decode_to_file(&raw_for_decode, sec, &final_for_decode, format)
    })
    .await
    .map_err(|e| Error::Config(format!("flatfiles: decode task panicked: {e}")))??;

    // Step 4: scratch cleanup. A failure here is non-fatal — the user
    // gets the decoded file regardless, and the raw blob is mostly
    // useful for debugging.
    let _ = std::fs::remove_file(&raw_path);

    Ok(final_path)
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

    let to_usize = |v: u64, field: &str| {
        usize::try_from(v).map_err(|_| {
            Error::Config(format!(
                "flatfiles: header field {field}={v} does not fit in usize on this target"
            ))
        })
    };
    let index_start = to_usize(hdr.index_offset, "index_offset")?;
    let index_byte_len = to_usize(hdr.index_byte_len, "index_byte_len")?;
    let data_byte_len = to_usize(hdr.data_byte_len, "data_byte_len")?;
    let index_end = index_start.checked_add(index_byte_len).ok_or_else(|| {
        Error::Config(format!(
            "flatfiles: header lengths overflow usize (index_offset={index_start}, index_byte_len={index_byte_len})"
        ))
    })?;
    let data_start = index_end;
    let data_end = data_start.checked_add(data_byte_len).ok_or_else(|| {
        Error::Config(format!(
            "flatfiles: header lengths overflow usize (data_start={data_start}, data_byte_len={data_byte_len})"
        ))
    })?;
    if data_end > blob.len() {
        return Err(Error::Config(format!(
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
        let bs = entry.block_start as usize;
        let be = entry.block_end as usize;
        if be > data_bytes.len() || bs > be {
            return Err(Error::Config(format!(
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
    // Per-invocation unique scratch path. Two concurrent calls for the
    // same `(sec, req, date)` must not share a file — they would race on
    // truncation and produce corrupt rows.
    let scratch = std::env::temp_dir().join(format!(
        "thetadatadx-flatfiles-{}-{}-{}-{}.raw",
        sec.as_wire(),
        req_name(req),
        date,
        uuid::Uuid::new_v4().simple()
    ));
    flatfile_request_raw(creds, sec, req, date, &scratch).await?;
    let scratch_for_decode = scratch.clone();
    let rows = tokio::task::spawn_blocking(move || decode_to_memory(&scratch_for_decode, sec))
        .await
        .map_err(|e| Error::Config(format!("flatfiles: decode task panicked: {e}")))??;
    let _ = std::fs::remove_file(&scratch);
    Ok(rows)
}

/// Decode an already-captured raw blob into a typed in-memory `Vec`.
pub(crate) fn decode_to_memory(raw_path: &Path, sec: SecType) -> Result<Vec<FlatFileRow>, Error> {
    let blob = std::fs::read(raw_path)?;
    let hdr = parse_header(&blob)?;

    let to_usize = |v: u64, field: &str| {
        usize::try_from(v).map_err(|_| {
            Error::Config(format!(
                "flatfiles: header field {field}={v} does not fit in usize on this target"
            ))
        })
    };
    let index_start = to_usize(hdr.index_offset, "index_offset")?;
    let index_byte_len = to_usize(hdr.index_byte_len, "index_byte_len")?;
    let data_byte_len = to_usize(hdr.data_byte_len, "data_byte_len")?;
    let index_end = index_start
        .checked_add(index_byte_len)
        .ok_or_else(|| Error::Config("flatfiles: header lengths overflow usize".into()))?;
    let data_start = index_end;
    let data_end = data_start
        .checked_add(data_byte_len)
        .ok_or_else(|| Error::Config("flatfiles: header lengths overflow usize".into()))?;
    if data_end > blob.len() {
        return Err(Error::Config(format!(
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
        let bs = entry.block_start as usize;
        let be = entry.block_end as usize;
        if be > data_bytes.len() || bs > be {
            return Err(Error::Config(format!(
                "flatfiles: INDEX block bounds [{bs}, {be}) escape DATA section ({} bytes)",
                data_bytes.len()
            )));
        }
        let block = &data_bytes[bs..be];
        decode_block(block, n_columns, &mut rows_buf)?;
        for row in &rows_buf {
            let divisor = crate::flatfiles::writer::price_divisor(row, hdr.price_type_idx);
            out.push(FlatFileRow::from_decoded(
                &entry.root,
                entry.exp,
                entry.strike,
                entry.right,
                &hdr.fmt,
                row,
                &data_idx,
                divisor,
            ));
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
}
