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
//! The FIT codec itself (`tdbe::codec::fit::FitReader` +
//! `tdbe::codec::fit::apply_deltas`) is shared with the FPSS surface so
//! we get exact wire-format parity for free.

use tdbe::codec::fit::{apply_deltas, FitReader};

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
    if block.is_empty() || n_columns == 0 {
        return Ok(());
    }

    let mut reader = FitReader::new(block);
    let mut prev: Vec<i32> = vec![0; n_columns];
    let mut alloc: Vec<i32> = vec![0; n_columns];
    let mut have_absolute = false;

    while !reader.is_exhausted() {
        alloc.iter_mut().for_each(|v| *v = 0);
        let n = reader.read_changes(&mut alloc[..]);
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
    fn date_marker_is_skipped() {
        // DATE marker row, then absolute "7"
        let mut buf = vec![0xCE, pack(1, END)]; // CE prefix + minimal end → consumed as DATE row
        buf.extend_from_slice(&[pack(7, END)]);
        let mut out = Vec::new();
        decode_block(&buf, 1, &mut out).unwrap();
        assert_eq!(out, vec![vec![7]]);
    }
}
