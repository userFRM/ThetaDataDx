//! INDEX walker for the raw FLATFILES blob.
//!
//! # On-disk layout (recovered from live wire bytes + bytecode shape)
//!
//! The raw blob the wire layer accumulates is one contiguous stream:
//!
//! ```text
//! offset 0   :  i32 BE  fmt_count          number of column DataType codes
//!         4 :  fmt_count × i32 BE          DataType.code() per column
//!         …  :  i64 BE  index_byte_len     bytes of INDEX after this header
//!         …  :  i64 BE  data_byte_len      bytes of DATA after the INDEX
//!         …  :  index_byte_len bytes      INDEX entries (see below)
//!         …  :  data_byte_len  bytes      DATA — concatenated FIT blocks
//! ```
//!
//! All multi-byte integers are big-endian — Java's `ByteBuffer.wrap(...)`
//! defaults to network order.
//!
//! # INDEX entries
//!
//! Each entry occupies a variable-length record:
//!
//! ```text
//!   u16 BE  entry_size                     bytes of entry_payload
//!   entry_payload  (entry_size bytes)      sec-type-dependent contract key
//!   i32 BE  block_volume                   "volume" hint (unused by SDK)
//!   i64 BE  block_start                    byte offset into DATA section
//!   i64 BE  block_end                      one past the last byte (exclusive)
//! ```
//!
//! The contract key inside `entry_payload` is:
//!
//! - Option:
//!   ```text
//!     u8 root_len ; root_utf8 ; i32 BE exp ; u8 right ; i32 BE strike ; i32 BE date
//!   ```
//!   `right` is the ASCII byte `'C'` (0x43) or `'P'` (0x50). `exp` and
//!   `date` are `YYYYMMDD` integers; `strike` is in tenths of a cent
//!   (vendor convention — strike `210000` ≡ $210.00).
//! - Stock:
//!   ```text
//!     u8 root_len ; root_utf8 ; i32 BE date
//!   ```
//!
//! The DATA section at `[block_start..block_end]` is FIT-encoded for the
//! per-column schema given by the header `fmt_count` codes. Each row in
//! the block corresponds to exactly one tick for that contract; see the
//! [`crate::flatfiles::decode`] module for the per-block FIT walk.

use std::io::{Cursor, Read};

use crate::error::Error;
use crate::flatfiles::datatype::DataType;
use crate::flatfiles::types::SecType;

/// Decoded blob header.
pub(crate) struct BlobHeader {
    /// Per-column schema for every FIT row in the DATA section.
    pub(crate) fmt: Vec<DataType>,
    /// Index of the `PRICE_TYPE` column inside `fmt`, if present. Used to
    /// recover fractional prices from integer-encoded fields.
    pub(crate) price_type_idx: Option<usize>,
    /// Byte length of the INDEX section that follows the header.
    pub(crate) index_byte_len: u64,
    /// Byte length of the DATA section that follows the INDEX.
    pub(crate) data_byte_len: u64,
    /// Byte offset of the first INDEX byte in the original blob (i.e.
    /// the size of the header itself).
    pub(crate) index_offset: u64,
}

/// Parse the leading header — fmt_count, fmt codes, indexLen, dataLen.
///
/// On success returns the decoded header; on a too-short or malformed
/// header returns `Error::Config` with a descriptive message rather
/// than panicking, so callers (notably the integration tests) can
/// surface the failure cleanly.
pub(crate) fn parse_header(blob: &[u8]) -> Result<BlobHeader, Error> {
    let mut cur = Cursor::new(blob);
    let fmt_count = read_i32(&mut cur)?;
    if !(0..=4096).contains(&fmt_count) {
        return Err(Error::Config(format!(
            "flatfiles: fmt_count {fmt_count} out of plausible range"
        )));
    }
    let mut fmt = Vec::with_capacity(fmt_count as usize);
    for _ in 0..fmt_count {
        let code = read_i32(&mut cur)?;
        fmt.push(DataType::from_code(code));
    }
    let index_byte_len = read_i64(&mut cur)?;
    let data_byte_len = read_i64(&mut cur)?;
    if index_byte_len < 0 || data_byte_len < 0 {
        return Err(Error::Config(format!(
            "flatfiles: negative section length(s) — index={index_byte_len}, data={data_byte_len}"
        )));
    }
    let price_type_idx = fmt.iter().position(|c| matches!(c, DataType::PriceType));
    Ok(BlobHeader {
        fmt,
        price_type_idx,
        index_byte_len: index_byte_len as u64,
        data_byte_len: data_byte_len as u64,
        index_offset: cur.position(),
    })
}

/// One INDEX entry's contract key + DATA-section pointer.
///
/// Field names follow the v3 vendor surface (`symbol`, `expiration`)
/// per the migration guide:
/// <https://docs.thetadata.us/Articles/Getting-Started/v2-migration-guide.html#_5-parameter-mapping>.
/// The wire layout still names the same bytes `root` / `exp` and the
/// parser locals below preserve that naming for diff-ability against
/// the upstream binary protocol.
#[derive(Debug, Clone)]
pub(crate) struct IndexEntry {
    /// UTF-8 ticker symbol (e.g. `"AAPL"`, `"SPY"`, `"ABBV"`).
    pub(crate) symbol: String,
    /// Option expiration (YYYYMMDD), or `None` for stock entries.
    pub(crate) expiration: Option<i32>,
    /// Strike price in tenths of a cent, or `None` for stocks.
    pub(crate) strike: Option<i32>,
    /// `'C'` or `'P'` for options, `None` for stocks.
    pub(crate) right: Option<char>,
    /// Byte offset in the DATA section where the entry's FIT block starts.
    pub(crate) block_start: u64,
    /// Byte offset one past the last byte of the FIT block.
    pub(crate) block_end: u64,
}

/// Iterator over INDEX entries.
///
/// Holds a borrowed slice of just the INDEX section bytes (i.e.
/// `&blob[hdr.index_offset .. hdr.index_offset + hdr.index_byte_len]`)
/// plus the sec-type-driven payload schema. Stops cleanly at end of
/// section; surfaces malformed entries as `Some(Err(_))`.
pub(crate) struct IndexIter<'a> {
    cur: Cursor<&'a [u8]>,
    sec: SecType,
}

impl<'a> IndexIter<'a> {
    pub(crate) fn new(index_bytes: &'a [u8], sec: SecType) -> Self {
        Self {
            cur: Cursor::new(index_bytes),
            sec,
        }
    }
}

impl<'a> Iterator for IndexIter<'a> {
    type Item = Result<IndexEntry, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let buf = *self.cur.get_ref();
        if self.cur.position() as usize >= buf.len() {
            return None;
        }
        Some(parse_one_entry(&mut self.cur, self.sec))
    }
}

fn parse_one_entry(cur: &mut Cursor<&[u8]>, sec: SecType) -> Result<IndexEntry, Error> {
    let entry_size = read_u16(cur)? as usize;
    let mut entry_buf = vec![0u8; entry_size];
    cur.read_exact(&mut entry_buf)?;
    let mut e = Cursor::new(&entry_buf[..]);

    let (root, exp, strike, right) = match sec {
        SecType::Option | SecType::Index => {
            // Index payload (when supported by the vendor) follows the
            // option layout — root_len, root, exp, right, strike, date.
            let root_len = read_u8(&mut e)? as usize;
            let mut root_bytes = vec![0u8; root_len];
            e.read_exact(&mut root_bytes)?;
            let root = String::from_utf8_lossy(&root_bytes).into_owned();
            let exp = read_i32(&mut e)?;
            let right_byte = read_u8(&mut e)?;
            let right = right_byte as char;
            let strike = read_i32(&mut e)?;
            // The trailing i32 is the contract's trading date; the row's
            // own DATE column carries the per-tick date and supersedes
            // it for CSV emission, so we consume but don't store.
            let _date = read_i32(&mut e)?;
            (root, Some(exp), Some(strike), Some(right))
        }
        SecType::Stock => {
            let root_len = read_u8(&mut e)? as usize;
            let mut root_bytes = vec![0u8; root_len];
            e.read_exact(&mut root_bytes)?;
            let root = String::from_utf8_lossy(&root_bytes).into_owned();
            let _date = read_i32(&mut e)?;
            (root, None, None, None)
        }
    };

    // Location: i32 volume, i64 start, i64 end. Volume is unused by the
    // SDK; we drop it but consume the bytes to advance the cursor.
    let _volume = read_i32(cur)?;
    let block_start = read_i64(cur)? as u64;
    let block_end = read_i64(cur)? as u64;
    Ok(IndexEntry {
        symbol: root,
        expiration: exp,
        strike,
        right,
        block_start,
        block_end,
    })
}

#[inline]
fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8, Error> {
    let mut b = [0u8; 1];
    cur.read_exact(&mut b)?;
    Ok(b[0])
}

#[inline]
fn read_u16(cur: &mut Cursor<&[u8]>) -> Result<u16, Error> {
    let mut b = [0u8; 2];
    cur.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

#[inline]
fn read_i32<R: Read>(cur: &mut R) -> Result<i32, Error> {
    let mut b = [0u8; 4];
    cur.read_exact(&mut b)?;
    Ok(i32::from_be_bytes(b))
}

#[inline]
fn read_i64<R: Read>(cur: &mut R) -> Result<i64, Error> {
    let mut b = [0u8; 8];
    cur.read_exact(&mut b)?;
    Ok(i64::from_be_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_header(fmt_codes: &[i32], index_len: i64, data_len: i64) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(fmt_codes.len() as i32).to_be_bytes());
        for c in fmt_codes {
            buf.extend_from_slice(&c.to_be_bytes());
        }
        buf.extend_from_slice(&index_len.to_be_bytes());
        buf.extend_from_slice(&data_len.to_be_bytes());
        buf
    }

    #[test]
    fn header_round_trip_open_interest_schema() {
        // Option open_interest schema observed in vendor CSV header:
        //   ms_of_day(1), open_interest(121), date(0)
        let raw = build_header(&[1, 121, 0], 100, 200);
        let hdr = parse_header(&raw).unwrap();
        assert_eq!(
            hdr.fmt,
            vec![DataType::MsOfDay, DataType::OpenInterest, DataType::Date]
        );
        assert!(hdr.price_type_idx.is_none()); // OI has no price column
        assert_eq!(hdr.index_byte_len, 100);
        assert_eq!(hdr.data_byte_len, 200);
        assert_eq!(hdr.index_offset, 4 + 4 * 3 + 16); // fmt_count + 3*i32 + 2*i64
    }

    #[test]
    fn header_locates_price_type_index() {
        // Trade schema includes PRICE_TYPE(4) at column 12 (vendor layout).
        let codes = [
            1, 131, 241, 242, 243, 244, 133, 132, 135, 134, 136, 137, 4, 138, 139, 0,
        ];
        let raw = build_header(&codes, 0, 0);
        let hdr = parse_header(&raw).unwrap();
        assert_eq!(hdr.price_type_idx, Some(12));
    }

    #[test]
    fn header_rejects_negative_lengths() {
        let raw = build_header(&[1, 0], -1, 100);
        assert!(parse_header(&raw).is_err());
    }

    #[test]
    fn header_rejects_truncated_input() {
        let raw = vec![0, 0, 0, 1, 0, 0, 0, 1]; // fmt_count=1, one code, no lengths
        assert!(parse_header(&raw).is_err());
    }

    #[test]
    fn option_index_entry_round_trip() {
        // Build one option entry: AAPL 20260117 P 200000 date=20260428
        // entry_size: 1 + 4 + 4 + 1 + 4 + 4 = 18
        let mut e = Vec::new();
        e.push(4u8); // root_len
        e.extend_from_slice(b"AAPL"); // root
        e.extend_from_slice(&20_260_117i32.to_be_bytes()); // exp
        e.push(b'P'); // right
        e.extend_from_slice(&200_000i32.to_be_bytes()); // strike
        e.extend_from_slice(&20_260_428i32.to_be_bytes()); // date

        let mut buf = Vec::new();
        buf.extend_from_slice(&(e.len() as u16).to_be_bytes());
        buf.extend_from_slice(&e);
        buf.extend_from_slice(&42i32.to_be_bytes()); // volume
        buf.extend_from_slice(&1000i64.to_be_bytes()); // start
        buf.extend_from_slice(&1500i64.to_be_bytes()); // end

        let mut iter = IndexIter::new(&buf, SecType::Option);
        let entry = iter.next().unwrap().unwrap();
        assert_eq!(entry.symbol, "AAPL");
        assert_eq!(entry.expiration, Some(20_260_117));
        assert_eq!(entry.strike, Some(200_000));
        assert_eq!(entry.right, Some('P'));
        assert_eq!(entry.block_start, 1000);
        assert_eq!(entry.block_end, 1500);
        assert!(iter.next().is_none());
    }

    #[test]
    fn stock_index_entry_round_trip() {
        // SPY date=20260428 — entry_size = 1 + 3 + 4 = 8
        let mut e = Vec::new();
        e.push(3u8);
        e.extend_from_slice(b"SPY");
        e.extend_from_slice(&20_260_428i32.to_be_bytes());

        let mut buf = Vec::new();
        buf.extend_from_slice(&(e.len() as u16).to_be_bytes());
        buf.extend_from_slice(&e);
        buf.extend_from_slice(&0i32.to_be_bytes());
        buf.extend_from_slice(&0i64.to_be_bytes());
        buf.extend_from_slice(&100i64.to_be_bytes());

        let mut iter = IndexIter::new(&buf, SecType::Stock);
        let entry = iter.next().unwrap().unwrap();
        assert_eq!(entry.symbol, "SPY");
        assert_eq!(entry.expiration, None);
        assert_eq!(entry.strike, None);
        assert_eq!(entry.right, None);
        assert_eq!(entry.block_start, 0);
        assert_eq!(entry.block_end, 100);
    }
}
