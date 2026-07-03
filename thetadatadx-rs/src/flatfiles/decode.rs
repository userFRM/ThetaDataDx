//! Per-contract FIT block decoder.
//!
//! Each INDEX entry points at a contiguous byte range inside the DATA
//! section; that range is FIT-encoded for a fixed per-blob column schema
//! (see [`crate::flatfiles::datatype`]). This module turns one such block
//! into an iterator of decoded `i32` rows ready for the format writers.
//!
//! Two FPSS-side facts hold here as well:
//! - The first row in a block is **absolute**; subsequent rows carry
//!   delta-compressed field updates.
//! - A leading `0xCE` byte in any row is a `DATE` marker — the row has no
//!   user-visible data and its end-nibble must be skipped.
//!
//! The FIT codec itself (`crate::tdbe::codec::fit::FitReader` +
//! `crate::tdbe::codec::fit::apply_deltas`) is shared with the FPSS surface so
//! we get exact wire-format parity for free.

use crate::tdbe::codec::fit::{apply_deltas, FitReader};

use crate::error::Error;

/// Iterate every row of a single contract block, applying FIT deltas
/// against the previous row.
///
/// `n_columns` is the per-blob schema width — one i32 per column.
/// Returns one `Vec<i32>` per row in `out`; `out` is **cleared** before
/// the iteration starts so callers can reuse the buffer across many
/// contracts without reallocating.
pub(crate) fn decode_block(
    block: &[u8],
    n_columns: usize,
    out: &mut Vec<Vec<i32>>,
) -> Result<(), Error> {
    out.clear();
    if block.is_empty() {
        return Ok(());
    }
    if n_columns == 0 {
        // A zero-column schema over non-empty DATA is a drifted header: every
        // row would decode to nothing while the block still carries bytes.
        // Fail loud rather than emit zero rows, matching the truncation guard.
        return Err(Error::decode_codec(
            "flatfiles: zero-column header with non-empty data",
        ));
    }

    let mut reader = FitReader::new(block);
    let mut prev: Vec<i32> = vec![0; n_columns];
    let mut alloc: Vec<i32> = vec![0; n_columns];
    let mut have_absolute = false;

    while !reader.is_exhausted() {
        alloc.iter_mut().for_each(|v| *v = 0);
        let n = reader.read_changes(&mut alloc[..]);
        if !reader.row_complete {
            // The block ran out before a terminating END nibble — a truncated
            // data row (partial integer flushed, trailing slots untrustworthy)
            // or a truncated DATE marker (returns n==0). An END-terminated row
            // always sets `row_complete`, so a false value here is always
            // truncation. The FPSS delta path rejects this same shape; match
            // it rather than accept the truncated block.
            return Err(Error::decode_codec(
                "flatfiles: FIT block truncated mid-row",
            ));
        }
        if n > n_columns {
            // Row carries more fields than the blob's column schema, so the
            // FIT reader already dropped the surplus into a silent clip. A
            // wider row is a drifted header; reject it rather than emit a
            // truncated row, matching the FPSS delta path's width guard.
            return Err(Error::decode_codec(
                "flatfiles: FIT row wider than header column count",
            ));
        }
        if reader.is_date {
            // DATE marker row — no user-visible data. Vendor's writer
            // skips DATE rows before they reach `toCSV2`, so we do too.
            continue;
        }
        if n == 0 {
            // Either an exhausted-buffer artefact or a zero-field row;
            // either way nothing to emit.
            continue;
        }
        if !have_absolute {
            // First absolute row — values stand on their own.
            prev.copy_from_slice(&alloc);
            have_absolute = true;
        } else {
            // Delta row — accumulate against `prev`. `n` is the number of
            // FIT-emitted fields for this row; trailing columns are
            // carried forward from `prev` (vendor `Tick.readID` parity).
            apply_deltas(&mut alloc, &prev, n);
            prev.copy_from_slice(&alloc);
        }
        out.push(alloc.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(high: u8, low: u8) -> u8 {
        (high << 4) | (low & 0x0F)
    }

    const FIELD_SEP: u8 = 0xB;
    const END: u8 = 0xD;

    #[test]
    fn empty_block_yields_no_rows() {
        let mut out = Vec::new();
        decode_block(&[], 5, &mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn single_absolute_row() {
        // "12,34<END>" → row = [12, 34]
        let buf = vec![pack(1, 2), pack(FIELD_SEP, 3), pack(4, END)];
        let mut out = Vec::new();
        decode_block(&buf, 2, &mut out).unwrap();
        assert_eq!(out, vec![vec![12, 34]]);
    }

    #[test]
    fn two_rows_apply_delta() {
        // Row 1 absolute: "100,50,200"
        // Row 2 delta:    "5,-3,10"  → [105, 47, 210]
        let row1 = [
            pack(1, 0),
            pack(0, FIELD_SEP),
            pack(5, 0),
            pack(FIELD_SEP, 2),
            pack(0, 0),
            pack(END, 0),
        ];
        let row2 = [
            pack(5, FIELD_SEP),
            pack(0xE, 3),
            pack(FIELD_SEP, 1),
            pack(0, END),
        ];
        let mut buf = Vec::new();
        buf.extend_from_slice(&row1);
        buf.extend_from_slice(&row2);

        let mut out = Vec::new();
        decode_block(&buf, 3, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], vec![100, 50, 200]);
        assert_eq!(out[1], vec![105, 47, 210]);
    }

    #[test]
    fn truncated_final_row_is_rejected() {
        // "12," then "34" with NO terminating END nibble: the block runs out
        // mid-row. The FPSS delta path rejects this shape; decode_block must
        // too, rather than push a garbage final row.
        let buf = vec![pack(1, 2), pack(FIELD_SEP, 3), pack(4, 0)];
        let mut out = Vec::new();
        let err = decode_block(&buf, 2, &mut out).unwrap_err();
        assert!(
            err.to_string().contains("truncated"),
            "expected a truncation decode error, got: {err}"
        );
    }

    #[test]
    fn truncated_date_marker_missing_end_is_rejected() {
        // A DATE marker byte (0xCE) with no terminating END nibble: the reader
        // returns n==0 and row_complete==false. That is still a truncated
        // block and must be rejected, not silently skipped as a DATE row.
        let buf = vec![0xCE, pack(1, 2)]; // marker + digits, no END
        let mut out = Vec::new();
        let err = decode_block(&buf, 2, &mut out).unwrap_err();
        assert!(
            err.to_string().contains("truncated"),
            "expected a truncation decode error, got: {err}"
        );
    }

    #[test]
    fn zero_column_header_with_data_is_rejected() {
        // A header claiming zero columns over a non-empty DATA block is a
        // drifted schema: silently decoding to zero rows would hide the drift.
        let buf = vec![pack(1, END)];
        let mut out = Vec::new();
        let err = decode_block(&buf, 0, &mut out).unwrap_err();
        assert!(
            err.to_string().contains("zero-column"),
            "expected a zero-column decode error, got: {err}"
        );
    }

    #[test]
    fn over_width_row_is_rejected() {
        // "12,34<END>" carries 2 fields against a 1-column schema: the extra
        // field would be silently clipped. The FPSS delta path rejects the
        // same width drift; decode_block must too.
        let buf = vec![pack(1, 2), pack(FIELD_SEP, 3), pack(4, END)];
        let mut out = Vec::new();
        let err = decode_block(&buf, 1, &mut out).unwrap_err();
        assert!(
            err.to_string().contains("wider"),
            "expected an over-width decode error, got: {err}"
        );
    }

    #[test]
    fn date_marker_is_skipped() {
        // DATE marker row, then absolute "7"
        let mut buf = vec![0xCE, pack(1, END)]; // CE prefix + minimal end → consumed as DATE row
        buf.extend_from_slice(&[pack(7, END)]);
        let mut out = Vec::new();
        decode_block(&buf, 1, &mut out).unwrap();
        assert_eq!(out, vec![vec![7]]);
    }
}
