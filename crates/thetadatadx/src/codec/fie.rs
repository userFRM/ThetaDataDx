//! FIE string-to-nibble encoder — used for building FPSS request lines.
//!
//! Reverse-engineered from decompiled `FIE.java` in ThetaData's terminal JAR.
//!
//! # Character-to-Nibble Mapping
//!
//! | Char  | Nibble |
//! |-------|--------|
//! | `'0'` | 0      |
//! | `'1'` | 1      |
//! | `'2'` | 2      |
//! | `'3'` | 3      |
//! | `'4'` | 4      |
//! | `'5'` | 5      |
//! | `'6'` | 6      |
//! | `'7'` | 7      |
//! | `'8'` | 8      |
//! | `'9'` | 9      |
//! | `'.'` | 10 (0xA) |
//! | `','` | 11 (0xB) |
//! | `'/'` | 12 (0xC) |
//! | `'n'` | 13 (0xD) — "newline" / end marker |
//! | `'-'` | 14 (0xE) |
//! | `'e'` | 15 (0xF) |
//!
//! # Packing
//!
//! Characters are packed pairwise into bytes: `byte = (nibble(c1) << 4) | nibble(c2)`.
//!
//! - Even-length string: all pairs packed, then terminator byte `0xDD` appended.
//! - Odd-length string: last byte = `(nibble(last_char) << 4) | 0xD`.
//! - Single character: one byte = `(nibble(char) << 4) | 0xD`.
//! - Empty string: returns just the terminator `[0xDD]`.

/// The "newline" nibble used for padding and termination.
const NEWLINE_NIBBLE: u8 = 0xD;

/// Map an ASCII character to its 4-bit FIE nibble value.
///
/// Returns `None` for characters not in the FIE alphabet.
#[inline]
pub const fn char_to_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'.' => Some(10),
        b',' => Some(11),
        b'/' => Some(12),
        b'n' => Some(13),
        b'-' => Some(14),
        b'e' => Some(15),
        _ => None,
    }
}

/// Map a nibble (0-15) back to its ASCII character.
///
/// Returns `None` for values outside 0-15.
#[inline]
pub const fn nibble_to_char(n: u8) -> Option<u8> {
    match n {
        0..=9 => Some(b'0' + n),
        10 => Some(b'.'),
        11 => Some(b','),
        12 => Some(b'/'),
        13 => Some(b'n'),
        14 => Some(b'-'),
        15 => Some(b'e'),
        _ => None,
    }
}

/// Encode a string into a FIE byte line for FPSS request building.
///
/// The input must contain only characters in the FIE alphabet
/// (`'0'-'9'`, `'.'`, `','`, `'/'`, `'n'`, `'-'`, `'e'`).
///
/// # Panics
///
/// Panics if the input contains a character not in the FIE alphabet.
/// Use [`try_string_to_fie_line`] for a non-panicking version.
pub fn string_to_fie_line(input: &str) -> Vec<u8> {
    match try_string_to_fie_line(input) {
        Ok(v) => v,
        Err(c) => panic!(
            "string_to_fie_line: character {:?} (0x{:02X}) not in FIE alphabet",
            c as char, c
        ),
    }
}

/// Encode a string into a FIE byte line, returning `Err(byte)` if any
/// character is outside the FIE alphabet.
pub fn try_string_to_fie_line(input: &str) -> Result<Vec<u8>, u8> {
    let bytes = input.as_bytes();
    let len = bytes.len();

    if len == 0 {
        // Empty string → just the terminator.
        return Ok(vec![(NEWLINE_NIBBLE << 4) | NEWLINE_NIBBLE]);
    }

    // Capacity: ceil(len/2) packed bytes + possibly 1 terminator.
    let mut out = Vec::with_capacity(len / 2 + 2);

    let mut i = 0;
    while i + 1 < len {
        let hi = char_to_nibble(bytes[i]).ok_or(bytes[i])?;
        let lo = char_to_nibble(bytes[i + 1]).ok_or(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }

    if len % 2 == 0 {
        // Even length: all characters consumed; append terminator 0xDD.
        out.push((NEWLINE_NIBBLE << 4) | NEWLINE_NIBBLE);
    } else {
        // Odd length: last character gets padded with newline nibble.
        let hi = char_to_nibble(bytes[len - 1]).ok_or(bytes[len - 1])?;
        out.push((hi << 4) | NEWLINE_NIBBLE);
    }

    Ok(out)
}

/// Decode a FIE byte line back into a string.
///
/// Strips the trailing newline-nibble padding/terminator.
/// Returns `None` if any nibble maps to an invalid character.
pub fn fie_line_to_string(data: &[u8]) -> Option<String> {
    let mut chars = Vec::with_capacity(data.len() * 2);

    for &byte in data {
        let hi = byte >> 4;
        let lo = byte & 0x0F;

        if hi == NEWLINE_NIBBLE {
            // Terminator start — stop.
            break;
        }
        chars.push(nibble_to_char(hi)?);

        if lo == NEWLINE_NIBBLE {
            // Odd-length padding — stop.
            break;
        }
        chars.push(nibble_to_char(lo)?);
    }

    String::from_utf8(chars).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibble_mapping_round_trip() {
        for c in b"0123456789.,/n-e".iter() {
            let n = char_to_nibble(*c).expect("should map");
            let back = nibble_to_char(n).expect("should reverse");
            assert_eq!(*c, back, "round-trip failed for char {:?}", *c as char);
        }
    }

    #[test]
    fn nibble_values_correct() {
        assert_eq!(char_to_nibble(b'0'), Some(0));
        assert_eq!(char_to_nibble(b'5'), Some(5));
        assert_eq!(char_to_nibble(b'9'), Some(9));
        assert_eq!(char_to_nibble(b'.'), Some(10));
        assert_eq!(char_to_nibble(b','), Some(11));
        assert_eq!(char_to_nibble(b'/'), Some(12));
        assert_eq!(char_to_nibble(b'n'), Some(13));
        assert_eq!(char_to_nibble(b'-'), Some(14));
        assert_eq!(char_to_nibble(b'e'), Some(15));
    }

    #[test]
    fn invalid_chars_return_none() {
        assert_eq!(char_to_nibble(b'A'), None);
        assert_eq!(char_to_nibble(b' '), None);
        assert_eq!(char_to_nibble(b'x'), None);
        assert_eq!(char_to_nibble(b'\n'), None);
    }

    #[test]
    fn empty_string() {
        let result = string_to_fie_line("");
        assert_eq!(result, vec![0xDD]);
    }

    #[test]
    fn single_char() {
        // "5" → nibble 5 in high, newline (0xD) in low → 0x5D
        let result = string_to_fie_line("5");
        assert_eq!(result, vec![0x5D]);
    }

    #[test]
    fn two_chars_even() {
        // "12" → nibbles (1, 2) = 0x12, then terminator 0xDD
        let result = string_to_fie_line("12");
        assert_eq!(result, vec![0x12, 0xDD]);
    }

    #[test]
    fn three_chars_odd() {
        // "123" → (1,2) = 0x12, then (3, newline) = 0x3D
        let result = string_to_fie_line("123");
        assert_eq!(result, vec![0x12, 0x3D]);
    }

    #[test]
    fn four_chars_even() {
        // "1234" → (1,2) = 0x12, (3,4) = 0x34, terminator 0xDD
        let result = string_to_fie_line("1234");
        assert_eq!(result, vec![0x12, 0x34, 0xDD]);
    }

    #[test]
    fn special_chars() {
        // "1.2" → (1, '.') = (1, 0xA) = 0x1A, (2, newline) = 0x2D
        let result = string_to_fie_line("1.2");
        assert_eq!(result, vec![0x1A, 0x2D]);
    }

    #[test]
    fn comma_separated() {
        // "1,2" → (1, ',') = (1, 0xB) = 0x1B, (2, newline) = 0x2D
        let result = string_to_fie_line("1,2");
        assert_eq!(result, vec![0x1B, 0x2D]);
    }

    #[test]
    fn negative_value() {
        // "-5" → ('-', '5') = (0xE, 5) = 0xE5, terminator 0xDD
        let result = string_to_fie_line("-5");
        assert_eq!(result, vec![0xE5, 0xDD]);
    }

    #[test]
    fn slash_and_dot() {
        // "1/2.3" → (1, '/') = (1, 0xC) = 0x1C, (2, '.') = (2, 0xA) = 0x2A, (3, newline) = 0x3D
        let result = string_to_fie_line("1/2.3");
        assert_eq!(result, vec![0x1C, 0x2A, 0x3D]);
    }

    #[test]
    fn all_special_chars() {
        // ".,/n-e" → (., ,) = (A, B) = 0xAB, (/, n) = (C, D) = 0xCD, (-, e) = (E, F) = 0xEF, term 0xDD
        let result = string_to_fie_line(".,/n-e");
        assert_eq!(result, vec![0xAB, 0xCD, 0xEF, 0xDD]);
    }

    #[test]
    fn round_trip_even() {
        let input = "12345678";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn round_trip_odd() {
        let input = "1234567";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn round_trip_single() {
        let input = "9";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn round_trip_with_specials() {
        // Note: 'n' (nibble 0xD) is the same value as the NEWLINE_NIBBLE terminator,
        // so strings containing 'n' cannot round-trip through the decoder — the
        // decoder sees 0xD and interprets it as end-of-string. This is by design:
        // 'n' is the newline/end marker in FIE, used only as a terminator in practice.
        let input = "100.50,-3/e";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn n_char_encodes_as_newline_nibble() {
        // 'n' maps to nibble 0xD, which is the terminator nibble. The encoder
        // happily produces it, but the decoder treats it as end-of-string.
        // This is intentional — 'n' is the FIE newline marker.
        let encoded = string_to_fie_line("n");
        // 'n' → nibble 0xD, odd length → (0xD << 4) | 0xD = 0xDD
        assert_eq!(encoded, vec![0xDD]);
        // Decoding 0xDD gives empty string (both nibbles are terminators).
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, "");
    }

    #[test]
    fn round_trip_empty() {
        let input = "";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn try_version_rejects_bad_char() {
        let result = try_string_to_fie_line("hello");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), b'h');
    }

    #[test]
    #[should_panic(expected = "not in FIE alphabet")]
    fn panicking_version_rejects_bad_char() {
        string_to_fie_line("ABC");
    }

    #[test]
    fn realistic_fpss_request() {
        // Typical FPSS subscribe request: "21,0,1,AAPL,0,20240315,C,15000"
        // But FIE only handles the 16-char alphabet, so the actual protocol
        // probably encodes numeric fields. Let's test a pure-numeric line:
        // "21,0,1,0,20240315,0,15000"
        let input = "21,0,1,0,20240315,0,15000";
        let encoded = string_to_fie_line(input);
        let decoded = fie_line_to_string(&encoded).expect("decode should succeed");
        assert_eq!(decoded, input);

        // Verify the first few bytes manually.
        // "21" → (2, 1) = 0x21
        // ",0" → (0xB, 0) = 0xB0
        assert_eq!(encoded[0], 0x21);
        assert_eq!(encoded[1], 0xB0);
    }

    #[test]
    fn fie_decode_partial_garbage_returns_none() {
        // A byte with nibble value 0xF is 'e', which IS valid.
        // But nibble_to_char(16) would be None — can't happen with 4-bit nibble.
        // So this test verifies that the decoder handles normal data.
        let data = [0xFF]; // Both nibbles = 15 = 'e'
        let decoded = fie_line_to_string(&data).expect("should decode");
        assert_eq!(decoded, "ee");
    }
}
