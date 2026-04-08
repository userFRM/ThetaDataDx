//! FIT nibble decoder — the core compression codec for FPSS tick data.
//!
//! Reverse-engineered from decompiled `FITReader.java` in `ThetaData`'s terminal JAR.
//!
//! # Wire Format
//!
//! Each byte encodes two nibbles: high (bits 7-4) and low (bits 3-0).
//!
//! | Nibble | Meaning                                                           |
//! |--------|-------------------------------------------------------------------|
//! | 0-9    | Decimal digit — accumulated left-to-right into current integer    |
//! | 0xB    | `FIELD_SEPARATOR` — flush integer to output, advance slot index     |
//! | 0xC    | `ROW_SEPARATOR` — flush, zero-fill slots to index 4, jump to 5      |
//! | 0xD    | END — flush current integer, terminate, return field count        |
//! | 0xE    | NEGATIVE — next flushed integer is negated                        |
//!
//! Digits accumulate into a buffer. On flush:
//! `value = digit[0]*10^(n-1) + digit[1]*10^(n-2) + ... + digit[n-1]`
//!
//! # Special Prefix
//!
//! If the first byte is `0xCE` (DATE marker), skip it and set `is_date = true`.
//! When a row ends after a DATE marker, `read_changes` returns 0.

/// Sentinel nibble values decoded from the FIT wire format.
const FIELD_SEP: u8 = 0xB;
const ROW_SEP: u8 = 0xC;
const END: u8 = 0xD;
const NEGATIVE: u8 = 0xE;

/// The "spacing" constant from FITReader.java: after a `ROW_SEPARATOR`, the
/// field index jumps to this value (zero-filling slots 0..4, skipping to 5).
const SPACING: usize = 5;

/// Maximum number of decimal digits that can accumulate before a flush.
/// 10 digits covers the full range of i32 (`2_147_483_647` = 10 digits).
const MAX_DIGITS: usize = 10;

/// DATE marker byte (0xCE as unsigned). In Java's signed byte world this is -50.
const DATE_MARKER: u8 = 0xCE;

/// Decode a FIT buffer in bulk, returning all rows as `Vec<Vec<i32>>`.
///
/// This is a higher-level convenience that reads all rows from `buf` and
/// applies delta decompression, returning absolute values per row.
///
/// Each inner `Vec<i32>` has exactly `fields_per_row` elements (zero-padded).
#[must_use]
pub fn decode_fit_buffer_bulk(buf: &[u8], fields_per_row: usize) -> Vec<Vec<i32>> {
    let mut reader = FitReader::new(buf);
    let mut rows = Vec::new();
    let mut prev = vec![0i32; fields_per_row];
    let mut alloc = vec![0i32; fields_per_row];
    let mut first = true;

    while !reader.is_exhausted() {
        alloc.fill(0);
        let n = reader.read_changes(&mut alloc);
        if n == 0 {
            continue;
        }
        if first {
            prev.copy_from_slice(&alloc);
            first = false;
        } else {
            apply_deltas(&mut alloc, &prev, n);
            prev.copy_from_slice(&alloc);
        }
        rows.push(alloc.clone());
    }
    rows
}

/// Stateful FIT stream reader.
///
/// Holds a position cursor into a byte buffer and decodes one row at a time
/// via [`read_changes`]. The caller is responsible for delta-accumulation
/// across rows (see module-level docs).
pub struct FitReader<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Set to `true` when the current row was preceded by a DATE marker (0xCE).
    pub is_date: bool,
}

impl<'a> FitReader<'a> {
    /// Create a new reader over `buf`, starting at byte offset 0.
    #[inline]
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            is_date: false,
        }
    }

    /// Create a new reader starting at an explicit byte offset.
    #[inline]
    #[must_use]
    pub fn with_offset(buf: &'a [u8], offset: usize) -> Self {
        Self {
            buf,
            pos: offset,
            is_date: false,
        }
    }

    /// Current byte position in the buffer.
    #[inline]
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns `true` when the cursor has reached or passed the end of the buffer.
    #[inline]
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Decode one row of FIT-encoded changes into `alloc`.
    ///
    /// Returns the number of fields written (i.e. `idx + 1` after the END nibble),
    /// or `0` if the row was a DATE marker row.
    ///
    /// Note: Java's `FITReader.readChanges()` returns `-1` on DATE marker rows,
    /// while this returns `0`. Callers use `is_date` to distinguish DATE rows
    /// from legitimate 0-field rows, making the difference inconsequential.
    ///
    /// # Panics
    ///
    /// Does **not** panic — if `alloc` is too short, excess fields are silently
    /// dropped (matching the Java behavior of writing to a pre-sized array that
    /// the caller controls).
    #[inline]
    pub fn read_changes(&mut self, alloc: &mut [i32]) -> usize {
        self.is_date = false;

        // Check for DATE marker prefix.
        if self.pos < self.buf.len() && self.buf[self.pos] == DATE_MARKER {
            self.is_date = true;
            self.pos += 1;
            // Consume until END nibble, then return 0.
            self.skip_to_end();
            return 0;
        }

        let mut idx: usize = 0;
        let mut digits = [0u8; MAX_DIGITS];
        let mut count: usize = 0;
        let mut negative = false;

        while self.pos < self.buf.len() {
            let byte = self.buf[self.pos];
            self.pos += 1;

            let high = byte >> 4;
            let low = byte & 0x0F;

            // Process high nibble, then low nibble.
            // Each returns true if the row has ended (END nibble encountered).
            if Self::process_nibble(
                high,
                alloc,
                &mut idx,
                &mut digits,
                &mut count,
                &mut negative,
            ) {
                return idx;
            }
            if Self::process_nibble(low, alloc, &mut idx, &mut digits, &mut count, &mut negative) {
                return idx;
            }
        }

        // Buffer exhausted without END nibble — flush whatever we have.
        if count > 0 || negative {
            let val = flush_digits(&digits, count, negative);
            if idx < alloc.len() {
                alloc[idx] = val;
            }
            idx += 1;
        }
        idx
    }

    /// Process a single nibble.
    ///
    /// Returns `true` if this nibble was an END marker (row complete).
    #[inline]
    fn process_nibble(
        nibble: u8,
        alloc: &mut [i32],
        idx: &mut usize,
        digits: &mut [u8; MAX_DIGITS],
        count: &mut usize,
        negative: &mut bool,
    ) -> bool {
        match nibble {
            0..=9 => {
                // Accumulate decimal digit.
                if *count < MAX_DIGITS {
                    digits[*count] = nibble;
                    *count += 1;
                }
                false
            }
            FIELD_SEP => {
                // Flush current integer, advance to next slot.
                let val = flush_digits(digits, *count, *negative);
                if *idx < alloc.len() {
                    alloc[*idx] = val;
                }
                *idx += 1;
                *count = 0;
                *negative = false;
                false
            }
            ROW_SEP => {
                // Flush current integer.
                let val = flush_digits(digits, *count, *negative);
                if *idx < alloc.len() {
                    alloc[*idx] = val;
                }
                *idx += 1;
                *count = 0;
                *negative = false;
                // Zero-fill up to index SPACING-1, then set idx to SPACING.
                while *idx < SPACING {
                    if *idx < alloc.len() {
                        alloc[*idx] = 0;
                    }
                    *idx += 1;
                }
                // Match Java: unconditionally reset to SPACING in case idx
                // was already >= SPACING before the zero-fill loop.
                *idx = SPACING;
                false
            }
            END => {
                // Flush and terminate.
                let val = flush_digits(digits, *count, *negative);
                if *idx < alloc.len() {
                    alloc[*idx] = val;
                }
                *idx += 1;
                // *count = 0;  // Not needed — we're done.
                true
            }
            NEGATIVE => {
                *negative = true;
                false
            }
            _ => {
                // Nibble 0xA (10) and 0xF (15) are unused in the FIT decode path.
                // Silently ignore, matching Java behavior.
                false
            }
        }
    }

    /// Consume bytes until an END nibble is found (used after DATE marker).
    fn skip_to_end(&mut self) {
        while self.pos < self.buf.len() {
            let byte = self.buf[self.pos];
            self.pos += 1;
            if (byte >> 4) == END || (byte & 0x0F) == END {
                return;
            }
        }
    }
}

/// Convert accumulated decimal digits to an i32 value.
///
/// `digits[0..count]` are decimal digits (0-9), accumulated left-to-right.
/// Result = digit[0] * 10^(count-1) + digit[1] * 10^(count-2) + ... + digit[count-1].
/// If `negative`, the result is negated.
///
/// Uses an i64 accumulator internally to avoid overflow for 10-digit values
/// near `i32::MAX`. Values that exceed i32 range are saturated.
///
/// An empty digit buffer (count == 0) flushes as 0 (matching Java behavior where
/// a separator immediately after another separator emits 0).
#[inline]
fn flush_digits(digits: &[u8; MAX_DIGITS], count: usize, negative: bool) -> i32 {
    let mut val: i64 = 0;
    for &digit in digits.iter().take(count) {
        val = val * 10 + i64::from(digit);
    }
    if negative {
        val = -val;
    }
    // Saturate to i32 range if the accumulated value overflows.
    i32::try_from(val).unwrap_or(if val > 0 { i32::MAX } else { i32::MIN })
}

/// Apply delta decompression to a tick row.
///
/// `n_fields` is the number of fields returned by `read_changes` for this row.
/// For fields `0..n_fields`, `tick[i] += prev[i]` (delta accumulation).
/// For fields `n_fields..total_slots`, `tick[i] = prev[i]` (carry forward).
///
/// The first row of a stream should be passed through without calling this
/// (its values are absolute).
///
/// Uses `wrapping_add` for protocol parity with Java, where `int` addition
/// wraps silently on overflow. This ensures identical bit patterns for
/// extreme delta values near i32 boundaries.
#[inline]
pub fn apply_deltas(tick: &mut [i32], prev: &[i32], n_fields: usize) {
    let len = tick.len().min(prev.len());
    let n = n_fields.min(len);
    for i in 0..n {
        tick[i] = tick[i].wrapping_add(prev[i]);
    }
    // Carry forward remaining fields from previous tick.
    if n < len {
        tick[n..len].copy_from_slice(&prev[n..len]);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: pack two nibbles into a byte.
    fn pack(high: u8, low: u8) -> u8 {
        (high << 4) | (low & 0x0F)
    }

    #[test]
    fn flush_digits_basic() {
        let mut d = [0u8; MAX_DIGITS];
        // Empty → 0
        assert_eq!(flush_digits(&d, 0, false), 0);
        // Single digit 7
        d[0] = 7;
        assert_eq!(flush_digits(&d, 1, false), 7);
        assert_eq!(flush_digits(&d, 1, true), -7);
        // 123
        d[0] = 1;
        d[1] = 2;
        d[2] = 3;
        assert_eq!(flush_digits(&d, 3, false), 123);
        assert_eq!(flush_digits(&d, 3, true), -123);
    }

    #[test]
    fn single_field_end() {
        // Encode "42" then END.
        // High nibble 4, low nibble 2 → byte 0x42
        // High nibble END (0xD), low nibble 0 (don't care) → byte 0xD0
        let data = [pack(4, 2), pack(END, 0)];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], 42);
    }

    #[test]
    fn two_fields_comma_then_end() {
        // "12,34\n"
        // 1, 2 → 0x12
        // COMMA(B), 3 → 0xB3
        // 4, END(D) → 0x4D
        let data = [pack(1, 2), pack(FIELD_SEP, 3), pack(4, END)];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 2);
        assert_eq!(alloc[0], 12);
        assert_eq!(alloc[1], 34);
    }

    #[test]
    fn negative_value() {
        // "-5\n"
        // NEGATIVE(E), 5 → 0xE5
        // END(D), 0 → 0xD0
        let data = [pack(NEGATIVE, 5), pack(END, 0)];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], -5);
    }

    #[test]
    fn negative_multi_digit() {
        // "-123,45\n"
        // NEGATIVE(E), 1 → 0xE1
        // 2, 3 → 0x23
        // COMMA(B), 4 → 0xB4
        // 5, END(D) → 0x5D
        let data = [
            pack(NEGATIVE, 1),
            pack(2, 3),
            pack(FIELD_SEP, 4),
            pack(5, END),
        ];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 2);
        assert_eq!(alloc[0], -123);
        assert_eq!(alloc[1], 45);
    }

    #[test]
    fn row_separator_zero_fills() {
        // "7/99\n"
        // 7, SLASH(C) → 0x7C
        // 9, 9 → 0x99
        // END(D), 0 → 0xD0
        //
        // Expected: alloc[0]=7, alloc[1..4]=0, alloc[5]=99 → n=6
        let data = [pack(7, ROW_SEP), pack(9, 9), pack(END, 0)];
        let mut alloc = [0i32; 16];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 6);
        assert_eq!(alloc[0], 7);
        assert_eq!(alloc[1], 0);
        assert_eq!(alloc[2], 0);
        assert_eq!(alloc[3], 0);
        assert_eq!(alloc[4], 0);
        assert_eq!(alloc[5], 99);
    }

    #[test]
    fn empty_fields_flush_as_zero() {
        // ",," → two commas then END = three fields, all zero.
        // COMMA(B), COMMA(B) → 0xBB
        // END(D), 0 → 0xD0
        let data = [pack(FIELD_SEP, FIELD_SEP), pack(END, 0)];
        let mut alloc = [99i32; 8]; // Fill with sentinel to verify overwrites.
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 3);
        assert_eq!(alloc[0], 0);
        assert_eq!(alloc[1], 0);
        assert_eq!(alloc[2], 0);
    }

    #[test]
    fn date_marker_returns_zero() {
        // DATE marker prefix (0xCE) followed by some content and END.
        // 0xCE, then anything, then a byte containing END nibble.
        let data = [DATE_MARKER, pack(1, 2), pack(END, 0)];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 0);
        assert!(reader.is_date);
    }

    #[test]
    fn multi_row_sequential() {
        // Two rows back-to-back.
        // Row 1: "5,3\n"   → 5, COMMA, 3, END
        // Row 2: "1,2\n"   → 1, COMMA, 2, END
        let data = [
            pack(5, FIELD_SEP),
            pack(3, END),
            pack(1, FIELD_SEP),
            pack(2, END),
        ];
        let mut alloc = [0i32; 8];
        let mut reader = FitReader::new(&data);

        let n1 = reader.read_changes(&mut alloc);
        assert_eq!(n1, 2);
        assert_eq!(alloc[0], 5);
        assert_eq!(alloc[1], 3);

        let mut alloc2 = [0i32; 8];
        let n2 = reader.read_changes(&mut alloc2);
        assert_eq!(n2, 2);
        assert_eq!(alloc2[0], 1);
        assert_eq!(alloc2[1], 2);
    }

    #[test]
    fn delta_decompression() {
        // First row absolute: [100, 200, 50]
        // Second row deltas:  [  5,  -3, 10]
        // After apply_deltas: [105, 197, 60]
        let prev = [100i32, 200, 50];
        let mut tick = [5i32, -3, 10];
        apply_deltas(&mut tick, &prev, 3);
        assert_eq!(tick, [105, 197, 60]);
    }

    #[test]
    fn delta_trailing_fields_carried_forward() {
        // prev has 5 fields, delta row only updates first 2.
        // Fields 2..5 are copied from prev.
        let prev = [10i32, 20, 30, 40, 50];
        let mut tick = [1i32, -2, 0, 0, 0];
        apply_deltas(&mut tick, &prev, 2);
        assert_eq!(tick, [11, 18, 30, 40, 50]);
    }

    #[test]
    fn large_value() {
        // Encode 1_999_999_999 → close to i32 max.
        // Digits: 1, 9, 9, 9, 9, 9, 9, 9, 9, 9
        let data = [
            pack(1, 9),
            pack(9, 9),
            pack(9, 9),
            pack(9, 9),
            pack(9, 9),
            pack(END, 0),
        ];
        let mut alloc = [0i32; 4];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], 1_999_999_999);
    }

    #[test]
    fn realistic_trade_tick_row() {
        // Simulate a trade tick: ms_of_day=34200000, seq=1, ext1-4=0, cond=0,
        // size=100, exchange=4, price=15025, cond_flags=0, price_flags=0,
        // volume_type=0, records_back=0, price_type=1, date=20240315
        //
        // FIT encoding (before ROW_SEP, fields 0-3 are ms/seq/ext1/ext2):
        //   "34200000,1/100,4,15025,,,,,1,20240315\n"
        //
        // Let's encode: 34200000 COMMA 1 SLASH 100 COMMA 4 COMMA 15025 COMMA
        //   COMMA COMMA COMMA COMMA 1 COMMA 20240315 END
        //
        // We'll build this byte-by-byte.
        let data = vec![
            // 34200000: digits 3,4,2,0,0,0,0,0
            pack(3, 4),
            pack(2, 0),
            pack(0, 0),
            pack(0, 0),
            // COMMA, 1
            pack(FIELD_SEP, 1),
            // SLASH (zero-fills ext1..ext4), then 100: 1,0,0
            pack(ROW_SEP, 1),
            pack(0, 0),
            // COMMA, 4
            pack(FIELD_SEP, 4),
            // COMMA, 15025: 1,5,0,2,5
            pack(FIELD_SEP, 1),
            pack(5, 0),
            pack(2, 5),
            // Five empty COMMAd fields (cond_flags, price_flags, volume_type,
            // records_back, then price_type=1)
            pack(FIELD_SEP, FIELD_SEP),
            pack(FIELD_SEP, FIELD_SEP),
            pack(FIELD_SEP, 1),
            // COMMA, 20240315: 2,0,2,4,0,3,1,5
            pack(FIELD_SEP, 2),
            pack(0, 2),
            pack(4, 0),
            pack(3, 1),
            pack(5, END),
        ];

        let mut alloc = [0i32; 32];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);

        // Fields: [0]=ms_of_day, [1]=seq, [2..4]=zero (ext1..ext4 from slash),
        //         [5]=size(100), [6]=exchange(4), [7]=price(15025),
        //         [8..11]=0 (empty commas), [12]=price_type(1), [13]=date(20240315)
        assert_eq!(n, 14);
        assert_eq!(alloc[0], 34200000); // ms_of_day
        assert_eq!(alloc[1], 1); // sequence
        assert_eq!(alloc[2], 0); // ext_condition1
        assert_eq!(alloc[3], 0); // ext_condition2
        assert_eq!(alloc[4], 0); // ext_condition3 (zero-filled by ROW_SEP)
        assert_eq!(alloc[5], 100); // size
        assert_eq!(alloc[6], 4); // exchange
        assert_eq!(alloc[7], 15025); // price
        assert_eq!(alloc[8], 0); // condition_flags
        assert_eq!(alloc[9], 0); // price_flags
        assert_eq!(alloc[10], 0); // volume_type
        assert_eq!(alloc[11], 0); // records_back
        assert_eq!(alloc[12], 1); // price_type
        assert_eq!(alloc[13], 20240315); // date
    }

    #[test]
    fn with_offset_starts_at_given_position() {
        // Prefix garbage [0xFF, 0xFF], then "5\n" starting at offset 2.
        let data = [0xFF, 0xFF, pack(5, END)];
        let mut alloc = [0i32; 4];
        let mut reader = FitReader::with_offset(&data, 2);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], 5);
        assert_eq!(reader.position(), 3);
    }

    #[test]
    fn exhausted_after_single_row() {
        let data = [pack(1, END)];
        let mut alloc = [0i32; 4];
        let mut reader = FitReader::new(&data);
        assert!(!reader.is_exhausted());
        reader.read_changes(&mut alloc);
        assert!(reader.is_exhausted());
    }

    #[test]
    fn zero_value_end() {
        // Just END immediately — should produce one field with value 0.
        let data = [pack(END, 0)];
        let mut alloc = [99i32; 4];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], 0);
    }

    #[test]
    fn negative_zero_flushes_as_zero() {
        // NEGATIVE then immediately END → -0 = 0.
        let data = [pack(NEGATIVE, END)];
        let mut alloc = [99i32; 4];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 1);
        assert_eq!(alloc[0], 0);
    }

    #[test]
    fn row_sep_with_empty_pre_slash_field() {
        // "/" immediately — empty field (value 0) before slash, then "8\n"
        // SLASH, 8 → 0xC8
        // END, 0 → 0xD0
        let data = [pack(ROW_SEP, 8), pack(END, 0)];
        let mut alloc = [99i32; 16];
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 6);
        assert_eq!(alloc[0], 0); // empty pre-slash field
        assert_eq!(alloc[1], 0); // zero-filled
        assert_eq!(alloc[2], 0);
        assert_eq!(alloc[3], 0);
        assert_eq!(alloc[4], 0);
        assert_eq!(alloc[5], 8);
    }

    #[test]
    fn delta_full_round_trip() {
        // Simulate a two-row FIT stream with delta decompression.
        // Row 1 (absolute): [1000, 50, 200]
        // Row 2 (delta):    [   5, -3,  10]
        // After delta:      [1005, 47, 210]

        // Encode row 1: "1000,50,200\n"
        let row1 = [
            pack(1, 0),
            pack(0, 0),
            pack(FIELD_SEP, 5),
            pack(0, FIELD_SEP),
            pack(2, 0),
            pack(0, END),
        ];
        // Encode row 2: "5,-3,10\n"
        let row2 = [
            pack(5, FIELD_SEP),
            pack(NEGATIVE, 3),
            pack(FIELD_SEP, 1),
            pack(0, END),
        ];
        let mut data = Vec::new();
        data.extend_from_slice(&row1);
        data.extend_from_slice(&row2);

        let mut reader = FitReader::new(&data);

        // Read row 1 (absolute).
        let mut prev = [0i32; 8];
        let n1 = reader.read_changes(&mut prev);
        assert_eq!(n1, 3);
        assert_eq!(prev[0], 1000);
        assert_eq!(prev[1], 50);
        assert_eq!(prev[2], 200);

        // Read row 2 (deltas).
        let mut tick = [0i32; 8];
        let n2 = reader.read_changes(&mut tick);
        assert_eq!(n2, 3);
        assert_eq!(tick[0], 5);
        assert_eq!(tick[1], -3);
        assert_eq!(tick[2], 10);

        // Apply deltas.
        apply_deltas(&mut tick, &prev, n2);
        assert_eq!(tick[0], 1005);
        assert_eq!(tick[1], 47);
        assert_eq!(tick[2], 210);
    }

    #[test]
    fn alloc_too_small_does_not_panic() {
        // More fields than alloc has space. Should not panic.
        // "1,2,3,4,5\n"
        let data = [
            pack(1, FIELD_SEP),
            pack(2, FIELD_SEP),
            pack(3, FIELD_SEP),
            pack(4, FIELD_SEP),
            pack(5, END),
        ];
        let mut alloc = [0i32; 2]; // Only room for 2.
        let mut reader = FitReader::new(&data);
        let n = reader.read_changes(&mut alloc);
        // Should return total field count (5), even though alloc only stored 2.
        assert_eq!(n, 5);
        assert_eq!(alloc[0], 1);
        assert_eq!(alloc[1], 2);
    }

    #[test]
    fn empty_buffer() {
        let data: [u8; 0] = [];
        let mut alloc = [0i32; 4];
        let mut reader = FitReader::new(&data);
        assert!(reader.is_exhausted());
        let n = reader.read_changes(&mut alloc);
        assert_eq!(n, 0);
    }
}
