//! gRPC length-prefix framing codec.
//!
//! The gRPC HTTP/2 wire format frames every message as
//! `[1 byte compressed flag][4 bytes big-endian length][payload bytes]`.
//! The codec emits and accepts only the uncompressed flag (`0`); a
//! compressed flag of `1` returns [`CodecError::CompressionUnsupported`].
//! See <https://grpc.io/docs/what-is-grpc/core-concepts/> for the
//! wire-format spec.
//!
//! [`Codec`] is phantom-typed on `<Req, Resp>` so callers cannot accidentally
//! decode a response with the wrong message type at compile time. Both
//! request and response types must implement [`prost::Message`].

use std::marker::PhantomData;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;
use thiserror::Error;

/// gRPC framing prefix length: 1 compressed flag + 4 big-endian length bytes.
pub(crate) const FRAME_HEADER_LEN: usize = 5;

/// Default upper bound on a single decoded frame, in bytes.
///
/// Matches the default tonic decoder ceiling so the in-house path does not
/// silently accept frames the existing tonic path would reject. Callers can
/// override per [`Codec`] instance via [`Codec::with_max_message_size`].
pub const DEFAULT_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Errors produced by the length-prefix codec.
///
/// All variants are owned (no borrowed wire bytes) so the error can be
/// propagated past the `Bytes` buffer that produced it.
///
/// `#[non_exhaustive]` so downstream `match` arms must include a
/// wildcard; new variants land without breaking semver.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CodecError {
    /// Compressed-flag byte in the frame prefix was `1`. The codec
    /// accepts only `grpc-encoding: identity` frames.
    #[error("gRPC frame is marked compressed but the codec only accepts identity-encoded frames")]
    CompressionUnsupported,
    /// Compressed-flag byte in the frame prefix was neither `0` nor `1`.
    /// gRPC reserves bits 1..=7 for future use; any non-zero, non-one
    /// value indicates a corrupt or hostile peer.
    #[error("gRPC frame has invalid compressed flag byte {0:#04x} (expected 0 or 1)")]
    InvalidCompressedFlag(u8),
    /// Length prefix declared a payload larger than the configured
    /// per-codec maximum.
    #[error("gRPC frame length {length} exceeds max message size {max}")]
    FrameTooLarge {
        /// Length the wire claims for this frame's payload.
        length: usize,
        /// Configured ceiling on this codec.
        max: usize,
    },
    /// `prost` failed to deserialize the payload into `Resp`. Carries the
    /// underlying message so callers can log it; the wire bytes are not
    /// retained.
    #[error("prost decode failed: {0}")]
    Decode(String),
    /// `prost` failed to encode the request into wire bytes. In
    /// practice unreachable on generated proto types — the encoder
    /// buffer is sized to `encoded_len` exactly — but propagated
    /// rather than silently dropped so a hand-written `Message`
    /// impl that returns `Err` fails the RPC cleanly instead of
    /// putting an empty frame on the wire.
    #[error("prost encode failed: {0}")]
    Encode(String),
}

/// Phantom-typed gRPC length-prefix codec.
///
/// One instance per `(Req, Resp)` pair; encoder takes `&Req`, decoder
/// produces `Resp`. The codec is stateless other than the configured
/// `max_message_size` ceiling.
pub struct Codec<Req, Resp> {
    max_message_size: usize,
    _marker: PhantomData<fn(Req) -> Resp>,
}

impl<Req, Resp> Default for Codec<Req, Resp> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Resp> Codec<Req, Resp> {
    /// Build a codec with the default 4 MiB per-frame ceiling.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            _marker: PhantomData,
        }
    }

    /// Build a codec with an explicit per-frame decode ceiling. Used by
    /// callers that want to mirror a configured `mdds.max_message_size`.
    #[must_use]
    pub const fn with_max_message_size(max_message_size: usize) -> Self {
        Self {
            max_message_size,
            _marker: PhantomData,
        }
    }

    /// Configured ceiling on a single decoded frame.
    #[must_use]
    pub const fn max_message_size(&self) -> usize {
        self.max_message_size
    }
}

impl<Req, Resp> Codec<Req, Resp>
where
    Req: Message,
{
    /// Encode `req` into a gRPC-framed [`Bytes`] payload.
    ///
    /// Layout: `[0u8][len: u32 big-endian][payload bytes]`. Always
    /// emits the uncompressed flag — the codec does not negotiate
    /// `grpc-encoding`.
    ///
    /// Allocates a single `BytesMut` sized for the encoded protobuf
    /// plus the 5-byte header; no intermediate copies.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Encode`] when `prost::Message::encode`
    /// fails. In practice this is unreachable on the generated
    /// proto types we route through here — the buffer is sized
    /// exactly to `encoded_len`, so prost only fails if it lies
    /// about the size (a prost bug) or the encoder is invoked
    /// against a hand-written `Message` impl that returns `Err`.
    /// Surfacing the error rather than silently dropping it lets
    /// the channel layer fail the RPC cleanly instead of putting
    /// a zero-length empty frame on the wire.
    pub fn encode(req: &Req) -> Result<Bytes, CodecError> {
        let payload_len = req.encoded_len();
        let mut buf = BytesMut::with_capacity(FRAME_HEADER_LEN + payload_len);
        buf.put_u8(0);
        // Saturating cast: payload_len > u32::MAX is unreachable in
        // practice (prost rejects messages above i32::MAX), but cap to
        // u32::MAX rather than panicking if a synthetic call ever
        // exceeds it. Frame size is also validated downstream by the
        // peer's max_message_size.
        let len_u32 = u32::try_from(payload_len).unwrap_or(u32::MAX);
        buf.put_u32(len_u32);
        // prost::Message::encode writes directly into the BytesMut tail;
        // BytesMut::with_capacity above is sized exactly so this never
        // reallocates. Surface any error untouched so the caller can
        // refuse to send the RPC.
        req.encode(&mut buf)
            .map_err(|e| CodecError::Encode(e.to_string()))?;
        Ok(buf.freeze())
    }
}

impl<Req, Resp> Codec<Req, Resp>
where
    Resp: Message + Default,
{
    /// Try to pop one decoded frame from the head of `buf`.
    ///
    /// Returns:
    /// - `Ok(Some(resp))` when one complete frame was consumed.
    /// - `Ok(None)` when `buf` does not yet hold a full frame; the
    ///   caller should append more wire bytes and try again.
    /// - `Err(_)` on a malformed frame; `buf` is **not** consumed and
    ///   the caller must drop the stream. This invariant covers every
    ///   error variant — header-level rejection (compressed flag,
    ///   oversized length) and payload-level rejection
    ///   ([`CodecError::Decode`]) alike.
    ///
    /// The decoder uses [`Bytes::split_to`] to take ownership of the
    /// payload slice on the success path, so the payload survives
    /// even after `buf` is reused for later frames.
    pub fn decode(&self, buf: &mut Bytes) -> Result<Option<Resp>, CodecError> {
        if buf.remaining() < FRAME_HEADER_LEN {
            return Ok(None);
        }

        // Peek (don't consume) the 5-byte prefix so a future poll can
        // re-read it once more wire bytes have arrived.
        let header = &buf[..FRAME_HEADER_LEN];
        let compressed_flag = header[0];
        match compressed_flag {
            0 => {}
            1 => return Err(CodecError::CompressionUnsupported),
            other => return Err(CodecError::InvalidCompressedFlag(other)),
        }

        // Length field is big-endian per the gRPC wire spec.
        let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        let payload_len = payload_len as usize;

        if payload_len > self.max_message_size {
            return Err(CodecError::FrameTooLarge {
                length: payload_len,
                max: self.max_message_size,
            });
        }

        if buf.remaining() < FRAME_HEADER_LEN + payload_len {
            // Header is here but payload is partial. Wait for more.
            return Ok(None);
        }

        // Decode against an immutable slice view first so a prost
        // failure leaves `buf` untouched — the caller's contract
        // ("Err(_) ⇒ buf is not consumed") then holds for every
        // error variant, not only the header-level ones above.
        let payload_slice = &buf[FRAME_HEADER_LEN..FRAME_HEADER_LEN + payload_len];
        let resp = Resp::decode(payload_slice).map_err(|e| CodecError::Decode(e.to_string()))?;

        // Decode succeeded — consume the header and payload bytes
        // from `buf` so the next call sees the following frame.
        buf.advance(FRAME_HEADER_LEN + payload_len);
        Ok(Some(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        data_value::DataType as DataValueType, DataValue, DataValueList, StockListSymbolsRequest,
        StockListSymbolsRequestQuery,
    };
    use proptest::prelude::*;

    fn make_request() -> StockListSymbolsRequest {
        StockListSymbolsRequest {
            query_info: None,
            params: Some(StockListSymbolsRequestQuery {}),
        }
    }

    fn make_response_with_strings(values: &[&str]) -> DataValueList {
        DataValueList {
            values: values
                .iter()
                .map(|s| DataValue {
                    data_type: Some(DataValueType::Text((*s).to_string())),
                })
                .collect(),
        }
    }

    #[test]
    fn encode_matches_tonic_wire_format_byte_for_byte() {
        // Hand-built reference of the tonic wire format (see
        // tonic 0.14 src/codec/encode.rs::finish_encoding): one
        // compressed flag byte (0 when not compressed) followed by
        // a 4-byte big-endian length, then the prost payload.
        //
        // This test pins the codec to the gRPC spec independent of
        // tonic's internals: if a future tonic release changes its
        // emission order or framing, our codec is anchored to the
        // wire bytes, not to tonic.
        let req = make_request();

        let mut reference = BytesMut::new();
        reference.put_u8(0); // compressed flag — uncompressed
        let payload = req.encode_to_vec();
        reference.put_u32(u32::try_from(payload.len()).unwrap());
        reference.extend_from_slice(&payload);
        let reference = reference.freeze();

        let ours = Codec::<StockListSymbolsRequest, DataValueList>::encode(&req)
            .expect("generated proto types always encode cleanly");
        assert_eq!(
            ours, reference,
            "in-house encoder matches the gRPC HTTP/2 wire spec byte-for-byte"
        );
    }

    #[test]
    fn encode_emits_5_byte_header_then_payload() {
        let req = make_request();
        let frame = Codec::<StockListSymbolsRequest, DataValueList>::encode(&req)
            .expect("generated proto types always encode cleanly");

        // Header byte 0 is the uncompressed flag.
        assert_eq!(frame[0], 0, "compressed flag must be 0");

        // Header bytes 1..5 are the big-endian payload length.
        let declared_len = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
        assert_eq!(declared_len as usize, frame.len() - FRAME_HEADER_LEN);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = make_response_with_strings(&["AAPL", "MSFT", "SPY"]);

        // Frame the response *as the server would*: wrap it in the
        // same 5-byte prefix and feed it to the decoder.
        let frame = Codec::<StockListSymbolsRequestQuery, DataValueList>::encode_response_for_test(
            &original,
        );

        let codec = Codec::<StockListSymbolsRequestQuery, DataValueList>::new();
        let mut buf = frame;
        let decoded = codec
            .decode(&mut buf)
            .expect("decode succeeds")
            .expect("frame is complete");

        assert_eq!(buf.remaining(), 0, "decoder consumed the whole frame");
        assert_eq!(decoded.values.len(), original.values.len());
        // Compare wire bytes (independent of struct equality / PartialEq).
        let mut a = Vec::new();
        let mut b = Vec::new();
        original.encode(&mut a).unwrap();
        decoded.encode(&mut b).unwrap();
        assert_eq!(a, b, "roundtrip preserves the protobuf wire bytes");
    }

    #[test]
    fn decode_returns_none_on_partial_header() {
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::new();

        // Only 3 of the 5 header bytes have arrived.
        let mut buf = Bytes::from_static(&[0, 0, 0]);
        let pre = buf.clone();
        let result = codec
            .decode(&mut buf)
            .expect("partial header is not an error");
        assert!(
            result.is_none(),
            "decoder returns None when header is short"
        );
        assert_eq!(buf, pre, "buf is unchanged on a None return");
    }

    #[test]
    fn decode_returns_none_on_partial_payload() {
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::new();

        // Header claims a 10-byte payload, but only 3 bytes followed.
        let mut buf = BytesMut::new();
        buf.put_u8(0);
        buf.put_u32(10);
        buf.extend_from_slice(&[1, 2, 3]);
        let mut buf = buf.freeze();

        let pre_len = buf.remaining();
        let result = codec
            .decode(&mut buf)
            .expect("partial payload is not an error");
        assert!(
            result.is_none(),
            "decoder returns None when payload is short"
        );
        assert_eq!(
            buf.remaining(),
            pre_len,
            "buf is unchanged on a None return"
        );
    }

    #[test]
    fn decode_rejects_compressed_flag() {
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::new();

        let mut buf = BytesMut::new();
        buf.put_u8(1); // compressed
        buf.put_u32(0);
        let mut buf = buf.freeze();

        let err = codec
            .decode(&mut buf)
            .expect_err("compressed flag rejected");
        assert!(matches!(err, CodecError::CompressionUnsupported));
    }

    #[test]
    fn decode_rejects_invalid_compressed_flag_byte() {
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::new();

        let mut buf = BytesMut::new();
        buf.put_u8(0xFF); // reserved bits set — invalid
        buf.put_u32(0);
        let mut buf = buf.freeze();

        let err = codec.decode(&mut buf).expect_err("invalid flag rejected");
        match err {
            CodecError::InvalidCompressedFlag(0xFF) => {}
            other => panic!("expected InvalidCompressedFlag(0xFF), got {other:?}"),
        }
    }

    #[test]
    fn decode_leaves_buf_unchanged_on_payload_decode_error() {
        // A legal 5-byte header followed by payload bytes that are
        // not valid protobuf for `DataValueList`. Production callers
        // rely on `Err(_) ⇒ buf is not consumed` so a corrupt frame
        // can be inspected (or simply dropped together with the
        // stream) without losing the framing offset.
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::new();

        // Build a frame that announces 4 bytes of payload but whose
        // payload is `[0xFF, 0xFF, 0xFF, 0xFF]` — every byte sets a
        // reserved wire tag, so `prost` rejects it as a malformed
        // message.
        let mut framed = BytesMut::new();
        framed.put_u8(0);
        framed.put_u32(4);
        framed.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        let mut buf = framed.freeze();

        let pre = buf.clone();
        let pre_len = buf.remaining();

        let err = codec
            .decode(&mut buf)
            .expect_err("malformed protobuf payload rejected");
        assert!(
            matches!(err, CodecError::Decode(_)),
            "expected CodecError::Decode, got {err:?}"
        );

        // Documented invariant: on Err the buffer is not consumed.
        // The caller drops the stream, but the framing offset and
        // every byte remain untouched.
        assert_eq!(
            buf.remaining(),
            pre_len,
            "buf length is unchanged after CodecError::Decode"
        );
        assert_eq!(buf, pre, "buf bytes are unchanged after CodecError::Decode");
    }

    #[test]
    fn decode_rejects_oversized_frame() {
        let codec = Codec::<StockListSymbolsRequest, DataValueList>::with_max_message_size(16);

        let mut buf = BytesMut::new();
        buf.put_u8(0);
        buf.put_u32(1_000_000); // claims a 1 MB payload, max is 16
        let mut buf = buf.freeze();

        let err = codec
            .decode(&mut buf)
            .expect_err("oversized frame rejected before allocating");
        match err {
            CodecError::FrameTooLarge { length, max } => {
                assert_eq!(length, 1_000_000);
                assert_eq!(max, 16);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn decode_two_frames_back_to_back() {
        let codec = Codec::<StockListSymbolsRequestQuery, DataValueList>::new();

        let first = make_response_with_strings(&["AAPL"]);
        let second = make_response_with_strings(&["MSFT", "GOOG"]);

        // Two framed responses concatenated, as the server emits on the
        // same h2 stream.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(
            &Codec::<StockListSymbolsRequestQuery, DataValueList>::encode_response_for_test(&first),
        );
        buf.extend_from_slice(
            &Codec::<StockListSymbolsRequestQuery, DataValueList>::encode_response_for_test(
                &second,
            ),
        );
        let mut buf = buf.freeze();

        let a = codec
            .decode(&mut buf)
            .unwrap()
            .expect("first frame complete");
        let b = codec
            .decode(&mut buf)
            .unwrap()
            .expect("second frame complete");
        assert!(
            codec.decode(&mut buf).unwrap().is_none(),
            "no third frame remains"
        );

        assert_eq!(a.values.len(), first.values.len());
        assert_eq!(b.values.len(), second.values.len());
    }

    proptest! {
        #[test]
        fn property_concatenated_frames_decode_back_to_originals(
            payloads in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 0..256),
                0..16,
            )
        ) {
            // Build a stream of `len(payloads)` frames where each frame
            // carries an arbitrary byte payload. We compose frames by
            // hand here rather than via `prost::Message::encode` so the
            // property covers raw payload bytes, not only well-formed
            // protobuf — the framing layer must not care about content.
            let mut stream = BytesMut::new();
            for p in &payloads {
                stream.put_u8(0);
                stream.put_u32(u32::try_from(p.len()).unwrap());
                stream.extend_from_slice(p);
            }
            let mut buf = stream.freeze();

            // Decode header-by-header using the raw frame extractor so
            // the property exercises the same length-prefix logic that
            // `Codec::decode` runs, independent of prost message shape.
            let mut decoded: Vec<Vec<u8>> = Vec::new();
            while let Some(frame) = pop_raw_frame_for_test(&mut buf, DEFAULT_MAX_MESSAGE_SIZE).unwrap() {
                decoded.push(frame.to_vec());
            }
            prop_assert_eq!(buf.remaining(), 0, "all bytes consumed");
            prop_assert_eq!(decoded.len(), payloads.len(), "frame count preserved");
            for (i, p) in payloads.iter().enumerate() {
                prop_assert_eq!(&decoded[i], p, "payload {} preserved", i);
            }
        }
    }

    // ── Test-only encoders / decoders ────────────────────────────────
    //
    // These wrap the same length-prefix logic the production `encode` /
    // `decode` methods use, exposed for tests that need to frame a
    // response (the codec only encodes requests in production) or pull
    // raw payload bytes (the property test compares raw bytes, not
    // prost-decoded messages).

    impl<Req, Resp> Codec<Req, Resp>
    where
        Resp: Message,
    {
        /// Test-only: frame `resp` exactly as a gRPC server would.
        /// Mirrors [`Codec::encode`] but takes the response type so
        /// roundtrip tests do not need a second codec instance.
        fn encode_response_for_test(resp: &Resp) -> Bytes {
            let payload_len = resp.encoded_len();
            let mut buf = BytesMut::with_capacity(FRAME_HEADER_LEN + payload_len);
            buf.put_u8(0);
            buf.put_u32(u32::try_from(payload_len).unwrap_or(u32::MAX));
            resp.encode(&mut buf)
                .expect("test buffer sized for payload");
            buf.freeze()
        }
    }

    /// Test-only: pop one raw payload slice from a framed stream,
    /// returning the payload bytes (header stripped). Returns `Ok(None)`
    /// when the buffer is short, errors on a malformed prefix.
    fn pop_raw_frame_for_test(
        buf: &mut Bytes,
        max_message_size: usize,
    ) -> Result<Option<Bytes>, CodecError> {
        if buf.remaining() < FRAME_HEADER_LEN {
            return Ok(None);
        }
        let header = &buf[..FRAME_HEADER_LEN];
        match header[0] {
            0 => {}
            1 => return Err(CodecError::CompressionUnsupported),
            other => return Err(CodecError::InvalidCompressedFlag(other)),
        }
        let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if payload_len > max_message_size {
            return Err(CodecError::FrameTooLarge {
                length: payload_len,
                max: max_message_size,
            });
        }
        if buf.remaining() < FRAME_HEADER_LEN + payload_len {
            return Ok(None);
        }
        buf.advance(FRAME_HEADER_LEN);
        Ok(Some(buf.split_to(payload_len)))
    }
}
