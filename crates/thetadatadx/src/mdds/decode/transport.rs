//! `ResponseData` decompression and `DataTable` decode.
//!
//! Recycles a thread-local zstd decompressor and output buffer so repeated
//! decompressions of similar-sized payloads avoid allocator pressure on the
//! working buffer.
//!
//! # `max_message_size` ceiling
//!
//! Every decode path threads the channel's configured
//! `max_message_size` ceiling into [`decode_data_table`] /
//! [`decompress_response`]. A hostile peer that sets
//! `ResponseData.original_size = i32::MAX` cannot trigger a 2 GiB
//! allocation: an `original_size` that exceeds the ceiling fails the
//! decode with [`Error::Decompress`]
//! (`DecompressErrorKind::MessageTooLarge`) before any `Vec::resize`
//! runs. Callers that don't have a ceiling (offline test fixtures)
//! call the `_unchecked` variant; production paths route through the
//! ceiling-aware variants.

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

/// Decompress a `ResponseData` payload using the default ceiling.
///
/// Equivalent to [`decompress_response_with_max`] with
/// [`crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE`]; new callers
/// should prefer the `_with_max` variant so they thread their
/// channel's configured ceiling through.
///
/// Takes the response by `&mut` so the identity-compression path can
/// move the payload `Vec` out instead of cloning.
///
/// # Errors
///
/// Returns [`Error::Decompress`] if the compression algorithm is
/// unknown, `original_size` exceeds the default ceiling, or zstd
/// decompression fails.
pub fn decompress_response(response: &mut proto::ResponseData) -> Result<Vec<u8>, Error> {
    decompress_response_with_max(response, crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE)
}

/// Decompress a `ResponseData` payload with a `max_message_size` ceiling.
///
/// The peer-advertised `ResponseData.original_size` is checked against
/// `max_message_size` BEFORE any `Vec::resize` runs so a hostile peer
/// (`original_size = i32::MAX`, ≈ 2 GiB) cannot trigger a runaway
/// allocation; the function returns `MessageTooLarge` first.
///
/// `max_message_size` mirrors the channel's
/// [`crate::grpc::codec::Codec::max_message_size`]. The frame-level
/// codec rejects oversized FRAMES on the wire; this guard rejects
/// oversized DECOMPRESSED PAYLOADS, which the codec cannot see
/// because `original_size` rides inside the compressed payload.
///
/// Prost's `.algo()` maps unknown enum values to `None`, so the
/// branch dispatches on the raw `i32` instead.
///
/// On the identity (`CompressionAlgo::None`) path the payload `Vec`
/// is moved out of `response.compressed_data` via `mem::take` — the
/// `&mut` borrow allows the move and the caller observes an empty
/// inner `Vec` after the call. The zstd path uses a thread-local
/// working buffer for the decompressed bytes.
///
/// # Errors
///
/// Returns [`Error::Decompress`] if the compression algorithm is
/// unknown, `original_size` exceeds `max_message_size`, or zstd
/// decompression fails.
// Reason: original_size is a protobuf u64 that fits in usize for valid payloads.
#[allow(clippy::cast_possible_truncation)]
pub fn decompress_response_with_max(
    response: &mut proto::ResponseData,
    max_message_size: usize,
) -> Result<Vec<u8>, Error> {
    let algo_raw = response
        .compression_description
        .as_ref()
        .map_or(0, |cd| cd.algo);

    match proto::CompressionAlgo::try_from(algo_raw) {
        Ok(proto::CompressionAlgo::None) => {
            // The uncompressed payload rides on the wire directly; the
            // gRPC codec already rejected the FRAME if it exceeded
            // `max_message_size`. We still range-check here so a
            // ResponseData synthesised from a non-gRPC source (test
            // fixtures, replay tools) cannot bypass the ceiling.
            if response.compressed_data.len() > max_message_size {
                return Err(Error::decompress_message_too_large(
                    response.compressed_data.len(),
                    max_message_size,
                ));
            }
            // Identity path: move the `Vec` out by value. The previous
            // `.clone()` allocated a fresh `Vec` of
            // `compressed_data.len()` bytes on every uncompressed
            // response; `mem::take` swaps the field for an empty `Vec`
            // (one-pointer write) and hands the original buffer to the
            // caller. Caller owns the response, so the take is
            // observable but harmless — every existing internal caller
            // discards the `ResponseData` immediately after the decode.
            Ok(std::mem::take(&mut response.compressed_data))
        }
        Ok(proto::CompressionAlgo::Zstd) => {
            // Reject hostile `original_size` BEFORE `Vec::resize`. The
            // protobuf wire field is `i32`; negative values fold to
            // `usize::MAX` via `try_from`, which also exceeds any
            // sane ceiling. A hostile peer's maximum is `i32::MAX`
            // (~2 GiB).
            let original_size = usize::try_from(response.original_size).unwrap_or(usize::MAX);
            if original_size > max_message_size {
                return Err(Error::decompress_message_too_large(
                    original_size,
                    max_message_size,
                ));
            }
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

/// Decode a `ResponseData` into a `DataTable` (legacy signature;
/// uses [`crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE`] as the
/// ceiling). Kept on the public API for backwards compatibility with
/// v10.0.x consumers; new callers should prefer
/// [`decode_data_table_with_max`].
///
/// # Errors
///
/// Returns [`Error::Decompress`] if decompression fails (including
/// `original_size > DEFAULT_MAX_MESSAGE_SIZE`) or [`Error::Decode`]
/// if protobuf deserialization fails.
pub fn decode_data_table(response: &mut proto::ResponseData) -> Result<proto::DataTable, Error> {
    decode_data_table_with_max(response, crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE)
}

/// Decode a `ResponseData` into a `DataTable`, honouring the channel's
/// `max_message_size` ceiling.
///
/// `max_message_size` is propagated from the originating
/// [`crate::grpc::Channel`] / [`crate::grpc::codec::Codec`]. Callers
/// without a channel-bound ceiling (offline tests, bench fixtures)
/// pass [`crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE`].
///
/// # Errors
///
/// Returns [`Error::Decompress`] if decompression fails (including
/// `original_size > max_message_size`) or [`Error::Decode`] if
/// protobuf deserialization fails.
pub fn decode_data_table_with_max(
    response: &mut proto::ResponseData,
    max_message_size: usize,
) -> Result<proto::DataTable, Error> {
    let bytes = decompress_response_with_max(response, max_message_size)?;
    let table: proto::DataTable = prost::Message::decode(bytes.as_slice())
        .map_err(|e| Error::decode_protobuf(e.to_string()))?;
    Ok(table)
}

#[cfg(test)]
mod r1_tests {
    use super::*;
    use crate::error::DecompressErrorKind;

    /// R1 [BLOCKER] proof: a hostile `ResponseData.original_size`
    /// larger than `max_message_size` returns a typed
    /// `MessageTooLarge` error BEFORE any allocation runs. Pinned at
    /// 2 GiB advertised vs 4 MiB ceiling — historically this triggered
    /// the runaway allocation; the fix returns a clean error.
    #[test]
    fn hostile_original_size_rejected_before_alloc() {
        let mut response = proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::Zstd as i32,
                level: 0,
            }),
            // 2 GiB advertised expansion — would have triggered a
            // `Vec::resize(usize::try_from(i32::MAX), 0)` before R1.
            // `original_size` is a wire-protocol `i32`; the v9 hostile
            // value `i32::MAX` is the upper bound a peer can set.
            original_size: i32::MAX,
            // Empty payload — never reached because original_size
            // fails the ceiling first.
            compressed_data: vec![],
        };
        let max = 4 * 1024 * 1024;
        let err =
            decompress_response_with_max(&mut response, max).expect_err("must reject hostile size");
        let Error::Decompress {
            kind: DecompressErrorKind::MessageTooLarge { size, max: ceiling },
            ..
        } = err
        else {
            panic!("expected MessageTooLarge, got {err:?}");
        };
        // Wire fixture advertised exactly i32::MAX; the rejection
        // surfaces that value verbatim against the configured
        // ceiling.
        assert_eq!(size, i32::MAX as usize);
        assert_eq!(ceiling, max);
    }

    /// Uncompressed-algo path is also size-guarded — a synthetic
    /// ResponseData with a 5 MiB `compressed_data` and the `None`
    /// algorithm cannot bypass the 4 MiB ceiling.
    #[test]
    fn hostile_uncompressed_payload_rejected() {
        let payload_bytes = 5 * 1024 * 1024;
        let mut response = proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::None as i32,
                level: 0,
            }),
            original_size: 0,
            // 5 MiB payload — exceeds the 4 MiB ceiling.
            compressed_data: vec![0_u8; payload_bytes],
        };
        let max = 4 * 1024 * 1024;
        let err = decompress_response_with_max(&mut response, max)
            .expect_err("must reject oversized payload");
        let Error::Decompress {
            kind: DecompressErrorKind::MessageTooLarge { size, max: ceiling },
            ..
        } = err
        else {
            panic!("expected MessageTooLarge, got {err:?}");
        };
        assert_eq!(size, payload_bytes);
        assert_eq!(ceiling, max);
    }

    /// A negative `original_size` (sign-flipped on the wire) folds to
    /// `usize::MAX` via `try_from` and is rejected without panicking.
    #[test]
    fn negative_original_size_rejected() {
        let mut response = proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::Zstd as i32,
                level: 0,
            }),
            original_size: -1,
            compressed_data: vec![],
        };
        let max = 4 * 1024 * 1024;
        let err = decompress_response_with_max(&mut response, max).expect_err("must reject");
        let Error::Decompress {
            kind: DecompressErrorKind::MessageTooLarge { size, max: ceiling },
            ..
        } = err
        else {
            panic!("expected MessageTooLarge, got {err:?}");
        };
        // i32::MIN .. -1 all fold to usize::MAX through
        // try_from(negative_i32). The exact saturation point is the
        // contract under test.
        assert_eq!(size, usize::MAX);
        assert_eq!(ceiling, max);
    }
}
