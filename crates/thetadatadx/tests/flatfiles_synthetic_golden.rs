//! Decoder-only golden test for the FLATFILES surface.
//!
//! Builds a synthetic raw blob (header + INDEX + FIT-encoded DATA) in
//! Rust against a hand-computed CSV expectation, then exercises the
//! same `decode_to_file` driver the live byte-match path uses. The
//! whole test runs in plain `cargo test` — no live wire, no env vars,
//! no vendor jar — so CI gets a hard regression gate on every push
//! whether or not the live byte-match step is wired up.
//!
//! Coverage: header parse, schema decoding, INDEX walk for an option
//! contract, FIT block decode (absolute row + delta row), CSV row
//! formatting, and the price formatter via `PRICE_TYPE` propagation.

use std::io::Write;

use thetadatadx::flatfiles::{FlatFileFormat, SecType};

/// Pack two 4-bit nibbles into one byte (high nibble first). Mirrors
/// the convention the FIT codec uses on the wire.
fn pack(high: u8, low: u8) -> u8 {
    (high << 4) | (low & 0x0F)
}

/// FIT field separator. Flushes the current integer to the output slot
/// and advances the slot index.
const FIELD_SEP: u8 = 0xB;
/// FIT row terminator. Flushes the current integer and ends the row.
const END: u8 = 0xD;

/// Build a minimal valid FLATFILES blob with one option contract and
/// two rows (one absolute, one FIT-delta) in the DATA section.
///
/// Schema: `[MsOfDay (1), Bid (103), PriceType (4), Date (0)]`
/// — a price-bearing schema so the test covers `fmt_price_into`.
fn synthetic_option_blob() -> Vec<u8> {
    // ----- Header -----
    let mut blob: Vec<u8> = Vec::new();
    let fmt_codes: [i32; 4] = [1, 103, 4, 0]; // ms_of_day, bid, price_type, date
    blob.write_all(&i32::to_be_bytes(fmt_codes.len() as i32))
        .unwrap();
    for c in fmt_codes {
        blob.write_all(&i32::to_be_bytes(c)).unwrap();
    }

    // We will splice index_byte_len and data_byte_len after we build the
    // sections, so reserve two i64 BE slots and remember their offsets.
    let index_len_pos = blob.len();
    blob.write_all(&i64::to_be_bytes(0)).unwrap();
    let data_len_pos = blob.len();
    blob.write_all(&i64::to_be_bytes(0)).unwrap();

    // ----- INDEX (one option entry pointing at the whole DATA section) -----
    let mut index: Vec<u8> = Vec::new();
    // Entry payload for an option: u8 root_len + root + i32 exp + u8 right
    // + i32 strike + i32 contract_date. The contract_date is unused by
    // the writer (per-row DATE column wins), so any value is fine.
    let root = b"SPY";
    let mut payload: Vec<u8> = Vec::new();
    payload.push(root.len() as u8);
    payload.extend_from_slice(root);
    payload.write_all(&i32::to_be_bytes(20240315)).unwrap(); // exp
    payload.push(b'C'); // right
    payload.write_all(&i32::to_be_bytes(580_000)).unwrap(); // strike (tenths of a cent → $580.00)
    payload.write_all(&i32::to_be_bytes(20240315)).unwrap(); // contract_date

    index
        .write_all(&u16::to_be_bytes(payload.len() as u16))
        .unwrap();
    index.extend_from_slice(&payload);
    index.write_all(&i32::to_be_bytes(0)).unwrap(); // volume hint (unused)

    // ----- DATA (FIT-encoded, two rows for the same contract) -----
    //
    // Row 1 absolute: ms_of_day=34_200_000, bid=12345 (price_type=8 →
    // 123.45), price_type=8, date=20_240_315.
    // Encoded as plain decimal nibbles separated by 0xB and terminated
    // with 0xD. 34_200_000 has 8 digits → bytes "3 4 2 0 0 0 0 0",
    // then FIELD_SEP (0xB), then "1 2 3 4 5", FIELD_SEP, "8", FIELD_SEP,
    // "2 0 2 4 0 3 1 5", END.
    //
    // Total nibbles for row 1:
    //   8 (ms_of_day) + 1 (sep) + 5 (bid) + 1 (sep) + 1 (pt) + 1 (sep)
    //   + 8 (date) + 1 (END) = 26 nibbles → 13 bytes.
    let row1_nibbles: [u8; 26] = [
        3, 4, 2, 0, 0, 0, 0, 0, FIELD_SEP, 1, 2, 3, 4, 5, FIELD_SEP, 8, FIELD_SEP, 2, 0, 2, 4, 0,
        3, 1, 5, END,
    ];
    let mut data: Vec<u8> = Vec::new();
    for w in row1_nibbles.chunks(2) {
        data.push(pack(w[0], w[1]));
    }
    // Row 2 delta: ms_of_day += 100, bid += 5, price_type unchanged,
    // date carried forward. Encoded as "100", FIELD_SEP, "5", END
    // (price_type and date columns are not emitted, so the FIT decoder
    // will return n=2 fields and `apply_deltas` carries the trailing
    // columns from the previous row).
    //   3 (100) + 1 (sep) + 1 (5) + 1 (END) = 6 nibbles → 3 bytes.
    let row2_nibbles: [u8; 6] = [1, 0, 0, FIELD_SEP, 5, END];
    for w in row2_nibbles.chunks(2) {
        data.push(pack(w[0], w[1]));
    }

    // INDEX needs block_start (i64) and block_end (i64) for this entry,
    // which we know now that DATA is sized.
    index.write_all(&i64::to_be_bytes(0)).unwrap(); // block_start
    index
        .write_all(&i64::to_be_bytes(data.len() as i64))
        .unwrap(); // block_end

    // Splice in the section lengths, then append INDEX and DATA.
    let index_len = index.len() as i64;
    let data_len = data.len() as i64;
    blob[index_len_pos..index_len_pos + 8].copy_from_slice(&i64::to_be_bytes(index_len));
    blob[data_len_pos..data_len_pos + 8].copy_from_slice(&i64::to_be_bytes(data_len));
    blob.extend_from_slice(&index);
    blob.extend_from_slice(&data);
    blob
}

/// Decode-only golden: the synthetic blob must produce this exact CSV.
///
/// Manual derivation:
/// - header columns (price_type column suppressed by the writer):
///   `symbol,expiration,strike,right,ms_of_day,bid,date`. The leading
///   contract-key column was renamed from `root` to `symbol` to match
///   the v3 vendor surface; see the migration guide:
///   <https://docs.thetadata.us/Articles/Getting-Started/v2-migration-guide.html#_5-parameter-mapping>.
/// - row 1: contract prefix `SPY,20240315,580000,C` plus
///   `34200000,123.45,20240315`. Bid=12345 with price_type=8 →
///   `12345 / 10^(10-8) = 12345 / 100 = 123.45`.
/// - row 2: contract prefix is the same; ms_of_day delta +100 →
///   34200100; bid delta +5 → 12350 → 123.5 (Rust f64 Display drops
///   the trailing zero); date carried forward → 20240315.
const EXPECTED_CSV: &str = "\
symbol,expiration,strike,right,ms_of_day,bid,date
SPY,20240315,580000,C,34200000,123.45,20240315
SPY,20240315,580000,C,34200100,123.5,20240315
";

#[test]
fn synthetic_blob_decodes_to_pinned_csv() {
    let blob = synthetic_option_blob();
    let dir = std::env::temp_dir().join(format!(
        "thetadatadx-flatfiles-synthetic-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let raw = dir.join("synthetic.bin");
    let csv = dir.join("synthetic.csv");
    std::fs::write(&raw, &blob).unwrap();

    thetadatadx::flatfiles::decoded_decode_to_file_for_test(
        &raw,
        SecType::Option,
        &csv,
        FlatFileFormat::Csv,
    )
    .expect("synthetic blob must decode cleanly");

    let got = std::fs::read_to_string(&csv).expect("read decoded CSV");
    assert_eq!(
        got, EXPECTED_CSV,
        "synthetic-blob CSV output drifted; left=actual right=expected\n--- actual ---\n{got}\n--- expected ---\n{EXPECTED_CSV}",
    );

    // Cleanup. Failures above keep the artefacts in place for triage.
    let _ = std::fs::remove_dir_all(&dir);
}
