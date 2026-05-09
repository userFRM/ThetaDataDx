//! `ResponseData` decompression and `DataTable` decode.
//!
//! Recycles a thread-local zstd decompressor and output buffer so repeated
//! decompressions of similar-sized payloads avoid allocator pressure on the
//! working buffer.

use std::cell::RefCell;

use crate::error::Error;
use crate::proto;

thread_local! {
    /// Reusable zstd decompressor **and** output buffer — avoids allocating both
    /// a fresh decompressor context and a fresh `Vec<u8>` on every call.
    ///
    /// The decompressor context (~128 KB of zstd internal state) is recycled, and
    /// the output buffer retains its capacity across calls so that repeated
    /// decompressions of similar-sized payloads hit no allocator at all.
    ///
    /// We use `decompress_to_buffer` which writes into the pre-existing Vec
    /// without reallocating when capacity is sufficient. The final `.clone()`
    /// is necessary since we return ownership, but the internal buffer capacity
    /// persists across calls — the key win is avoiding repeated alloc/dealloc
    /// cycles for the working buffer.
    static ZSTD_STATE: RefCell<(zstd::bulk::Decompressor<'static>, Vec<u8>)> = RefCell::new((
        // Infallible in practice: zstd decompressor creation only fails on OOM.
        // thread_local! does not support Result, so unwrap is intentional here.
        zstd::bulk::Decompressor::new().expect("zstd decompressor creation failed (possible OOM)"),
        Vec::with_capacity(1024 * 1024), // 1 MB initial capacity
    ));
}

/// Decompress a `ResponseData` payload. Returns the raw protobuf bytes of the `DataTable`.
///
/// # Unknown compression algorithms
///
/// Prost's `.algo()` silently maps unknown enum values to the default (None=0),
/// so we check the raw i32 to detect truly unknown algorithms. Without this,
/// an unrecognized algorithm would be treated as uncompressed, producing garbage.
///
/// # Buffer recycling
///
/// Uses a thread-local `(Decompressor, Vec<u8>)` pair. The `Vec` retains its
/// capacity across calls, so repeated decompressions of similar-sized payloads
/// avoid hitting the allocator for the working buffer. The returned `Vec<u8>`
/// is a clone (we must return ownership), but the internal slab persists.
/// # Errors
///
/// Returns [`Error::Decompress`] if the compression algorithm is unknown or
/// zstd decompression fails.
// Reason: original_size is a protobuf u64 that fits in usize for valid payloads.
#[allow(clippy::cast_possible_truncation)]
pub fn decompress_response(response: &proto::ResponseData) -> Result<Vec<u8>, Error> {
    let algo_raw = response
        .compression_description
        .as_ref()
        .map_or(0, |cd| cd.algo);

    match proto::CompressionAlgo::try_from(algo_raw) {
        Ok(proto::CompressionAlgo::None) => Ok(response.compressed_data.clone()),
        Ok(proto::CompressionAlgo::Zstd) => {
            let original_size = usize::try_from(response.original_size).unwrap_or(0);
            ZSTD_STATE.with(|cell| {
                let (ref mut dec, ref mut buf) = *cell.borrow_mut();
                buf.clear();
                buf.resize(original_size, 0);
                let n = dec
                    .decompress_to_buffer(&response.compressed_data, buf)
                    .map_err(|e| Error::decompress_zstd(e.to_string()))?;
                buf.truncate(n);
                Ok(buf.clone())
            })
        }
        _ => Err(Error::decompress_unknown_algorithm(algo_raw)),
    }
}

/// Decode a `ResponseData` into a `DataTable`.
///
/// # Errors
///
/// Returns [`Error::Decompress`] if decompression fails or [`Error::Decode`]
/// if protobuf deserialization fails.
pub fn decode_data_table(response: &proto::ResponseData) -> Result<proto::DataTable, Error> {
    let bytes = decompress_response(response)?;
    let table: proto::DataTable = prost::Message::decode(bytes.as_slice())
        .map_err(|e| Error::decode_protobuf(e.to_string()))?;
    Ok(table)
}
