//! gRPC status as the crate's own type.
//!
//! Every gRPC response — successful or not — ends with a status: a
//! numeric code, an optional human message, and optional
//! `grpc-status-details-bin` metadata carrying a `google.rpc.Status`
//! payload. See <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>
//! for the canonical code list.
//!
//! [`Status`] mirrors exactly the fields the crate consumes — code,
//! message, and the `google.rpc.RetryInfo` backoff hint — so no
//! third-party status type crosses the module boundary. The conversion
//! from the underlying stack's status type happens once, at
//! [`Status::from_tonic`], inside this module.

/// `grpc-status: 0` — the `Ok` code.
pub(crate) const STATUS_OK: u32 = 0;

/// Fully-qualified `Any.type_url` suffix for `google.rpc.RetryInfo`.
const RETRY_INFO_TYPE_URL_SUFFIX: &str = "google.rpc.RetryInfo";

/// gRPC status carried in response trailers (or a trailers-only
/// response head).
///
/// Stored as the raw numeric `code` so callers match against the gRPC
/// canonical codes directly; [`crate::error::GrpcStatusKind`] is the
/// typed public mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Numeric gRPC status code, e.g. `0` for `Ok`, `13` for `Internal`.
    code: u32,
    /// Human-readable status message (may be empty when the trailer is
    /// absent or the status is `Ok`).
    message: String,
    /// Server-supplied backoff hint decoded from the
    /// `google.rpc.RetryInfo` detail in `grpc-status-details-bin`, when
    /// present. Retry loops clamp their computed delay up to this value
    /// so a server-instructed cooldown is always honoured in full.
    retry_delay: Option<std::time::Duration>,
}

impl Status {
    /// Build a status with the given code and (possibly empty) message.
    #[must_use]
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_delay: None,
        }
    }

    /// Convert the underlying stack's status into the crate's own
    /// type. The numeric code and UTF-8 message map directly; the
    /// `grpc-status-details-bin` payload (already base64-decoded by
    /// the receive path) is scanned for a `google.rpc.RetryInfo`
    /// backoff hint.
    #[must_use]
    pub(crate) fn from_tonic(status: &tonic::Status) -> Self {
        // `tonic::Code` discriminants match the wire codes one-for-one;
        // the canonical range 0..=16 always fits u32.
        let code = u32::try_from(status.code() as i32).unwrap_or(u32::MAX);
        Self {
            code,
            message: status.message().to_string(),
            retry_delay: decode_retry_delay(status.details()),
        }
    }

    /// Numeric status code.
    #[must_use]
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// Status message; empty on `Ok` or when the trailer is absent.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Server-supplied minimum backoff before the next retry, decoded
    /// from the `google.rpc.RetryInfo` status detail. `None` when the
    /// server sent no hint (the common case).
    #[must_use]
    pub const fn retry_delay(&self) -> Option<std::time::Duration> {
        self.retry_delay
    }

    /// `true` iff the status code is `0` (gRPC `Ok`).
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.code == STATUS_OK
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "grpc-status={}", self.code)
        } else {
            write!(f, "grpc-status={}: {}", self.code, self.message)
        }
    }
}

// ─── google.rpc.RetryInfo decode ────────────────────────────────────
//
// Minimal local mirrors of the `google.rpc` protos involved in the
// RetryInfo hint. The crate does not vendor the google.rpc proto tree;
// the two messages below pin only the fields this parser reads, with
// tags matching the canonical definitions:
//
//   google.rpc.Status   { ... repeated google.protobuf.Any details = 3; }
//   google.rpc.RetryInfo { google.protobuf.Duration retry_delay = 1; }
//
// Unknown fields are skipped by prost, so richer detail payloads decode
// cleanly.

/// Local mirror of `google.rpc.Status` (details field only).
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatusProto {
    #[prost(message, repeated, tag = "3")]
    details: prost::alloc::vec::Vec<prost_types::Any>,
}

/// Local mirror of `google.rpc.RetryInfo`.
#[derive(Clone, PartialEq, prost::Message)]
struct RetryInfoProto {
    #[prost(message, optional, tag = "1")]
    retry_delay: Option<prost_types::Duration>,
}

/// Decode the `grpc-status-details-bin` payload (a serialized
/// `google.rpc.Status`, base64 already stripped by the receive path)
/// into the `google.rpc.RetryInfo.retry_delay` hint, if one is present.
///
/// Any malformed layer — non-proto payload, missing detail — degrades
/// to `None` rather than invalidating the status it travels with.
/// Negative durations are rejected, as is a `nanos` field outside the
/// canonical `[0, 1e9)` range — an out-of-range fractional component would
/// otherwise inflate the resulting delay past its intended value.
fn decode_retry_delay(raw: &[u8]) -> Option<std::time::Duration> {
    use prost::Message;
    if raw.is_empty() {
        return None;
    }
    let status = RpcStatusProto::decode(raw).ok()?;
    for any in &status.details {
        if !any.type_url.ends_with(RETRY_INFO_TYPE_URL_SUFFIX) {
            continue;
        }
        let info = RetryInfoProto::decode(any.value.as_slice()).ok()?;
        let proto_delay = info.retry_delay?;
        if proto_delay.seconds < 0 || proto_delay.nanos < 0 || proto_delay.nanos >= 1_000_000_000 {
            return None;
        }
        let secs = u64::try_from(proto_delay.seconds).ok()?;
        let nanos = u32::try_from(proto_delay.nanos).ok()?;
        return Some(std::time::Duration::new(secs, nanos));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    /// Encode a `google.rpc.Status` carrying one RetryInfo detail with
    /// the given delay — the details payload a server ships.
    fn retry_info_details(secs: i64, nanos: i32) -> Vec<u8> {
        let info = RetryInfoProto {
            retry_delay: Some(prost_types::Duration {
                seconds: secs,
                nanos,
            }),
        };
        let status = RpcStatusProto {
            details: vec![prost_types::Any {
                type_url: "type.googleapis.com/google.rpc.RetryInfo".to_string(),
                value: info.encode_to_vec(),
            }],
        };
        status.encode_to_vec()
    }

    #[test]
    fn display_omits_empty_message() {
        let s = Status::new(0, "");
        assert_eq!(s.to_string(), "grpc-status=0");
        let s = Status::new(13, "internal");
        assert_eq!(s.to_string(), "grpc-status=13: internal");
    }

    #[test]
    fn from_tonic_maps_every_canonical_code() {
        use tonic::Code;
        let cases = [
            (Code::Ok, 0u32),
            (Code::Cancelled, 1),
            (Code::Unknown, 2),
            (Code::InvalidArgument, 3),
            (Code::DeadlineExceeded, 4),
            (Code::NotFound, 5),
            (Code::AlreadyExists, 6),
            (Code::PermissionDenied, 7),
            (Code::ResourceExhausted, 8),
            (Code::FailedPrecondition, 9),
            (Code::Aborted, 10),
            (Code::OutOfRange, 11),
            (Code::Unimplemented, 12),
            (Code::Internal, 13),
            (Code::Unavailable, 14),
            (Code::DataLoss, 15),
            (Code::Unauthenticated, 16),
        ];
        for (code, wire) in cases {
            let s = Status::from_tonic(&tonic::Status::new(code, "m"));
            assert_eq!(s.code(), wire, "wire code mismatch for {code:?}");
            assert_eq!(s.message(), "m");
        }
    }

    /// Full-chain pin: the underlying stack's status converts through
    /// [`Status::from_tonic`] into `crate::Error::Grpc` with the
    /// matching [`crate::error::GrpcStatusKind`] for every canonical
    /// code, and the `RetryInfo` hint lands in `retry_after`.
    #[test]
    fn every_status_kind_maps_through_crate_error() {
        use crate::error::{Error, GrpcStatusKind};
        use tonic::Code;
        let cases = [
            (Code::Ok, GrpcStatusKind::Ok),
            (Code::Cancelled, GrpcStatusKind::Cancelled),
            (Code::Unknown, GrpcStatusKind::Unknown),
            (Code::InvalidArgument, GrpcStatusKind::InvalidArgument),
            (Code::DeadlineExceeded, GrpcStatusKind::DeadlineExceeded),
            (Code::NotFound, GrpcStatusKind::NotFound),
            (Code::AlreadyExists, GrpcStatusKind::AlreadyExists),
            (Code::PermissionDenied, GrpcStatusKind::PermissionDenied),
            (Code::ResourceExhausted, GrpcStatusKind::ResourceExhausted),
            (Code::FailedPrecondition, GrpcStatusKind::FailedPrecondition),
            (Code::Aborted, GrpcStatusKind::Aborted),
            (Code::OutOfRange, GrpcStatusKind::OutOfRange),
            (Code::Unimplemented, GrpcStatusKind::Unimplemented),
            (Code::Internal, GrpcStatusKind::Internal),
            (Code::Unavailable, GrpcStatusKind::Unavailable),
            (Code::DataLoss, GrpcStatusKind::DataLoss),
            (Code::Unauthenticated, GrpcStatusKind::Unauthenticated),
        ];
        for (code, expected_kind) in cases {
            let details = retry_info_details(1, 250_000_000);
            let upstream = tonic::Status::with_details(code, "wire message", details.into());
            let err = Error::from(Status::from_tonic(&upstream));
            match err {
                Error::Grpc {
                    kind,
                    message,
                    retry_after,
                } => {
                    assert_eq!(kind, expected_kind, "kind mismatch for {code:?}");
                    assert_eq!(message, "wire message");
                    assert_eq!(
                        retry_after,
                        Some(std::time::Duration::from_millis(1_250)),
                        "RetryInfo hint must survive the full mapping chain for {code:?}"
                    );
                }
                other => panic!("expected Error::Grpc for {code:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn retry_info_detail_surfaces_as_retry_delay() {
        let details = retry_info_details(2, 500_000_000);
        let status =
            tonic::Status::with_details(tonic::Code::ResourceExhausted, "cooldown", details.into());
        let s = Status::from_tonic(&status);
        assert_eq!(s.code(), 8);
        assert_eq!(
            s.retry_delay(),
            Some(std::time::Duration::from_millis(2_500)),
            "RetryInfo delay must surface on the converted status"
        );
    }

    #[test]
    fn absent_or_malformed_details_degrade_to_no_hint() {
        // No details at all.
        let s = Status::from_tonic(&tonic::Status::new(tonic::Code::Unavailable, "down"));
        assert_eq!(s.retry_delay(), None);

        // A non-proto details payload must not invalidate the status
        // it travels with.
        let status = tonic::Status::with_details(
            tonic::Code::Unavailable,
            "down",
            bytes::Bytes::from_static(b"\xff\xfe\xfd not a proto"),
        );
        let s = Status::from_tonic(&status);
        assert_eq!(s.code(), 14);
        assert_eq!(s.retry_delay(), None);
    }

    #[test]
    fn negative_retry_delay_is_rejected() {
        let details = retry_info_details(-1, 0);
        let status = tonic::Status::with_details(tonic::Code::Unavailable, "down", details.into());
        assert_eq!(
            Status::from_tonic(&status).retry_delay(),
            None,
            "a negative server hint must be discarded, not wrapped"
        );
    }

    #[test]
    fn out_of_range_nanos_is_rejected() {
        // A nanos field at or above 1e9 is outside the canonical Duration
        // range; carrying it through would inflate the delay (here by ~1.5s).
        let details = retry_info_details(0, 1_500_000_000);
        let status = tonic::Status::with_details(tonic::Code::Unavailable, "down", details.into());
        assert_eq!(
            Status::from_tonic(&status).retry_delay(),
            None,
            "an out-of-range fractional component must be discarded, not added"
        );
    }

    #[test]
    fn foreign_detail_types_are_skipped() {
        // A details list whose Any payload is NOT RetryInfo must be
        // ignored without error.
        let status_proto = RpcStatusProto {
            details: vec![prost_types::Any {
                type_url: "type.googleapis.com/google.rpc.ErrorInfo".to_string(),
                value: vec![1, 2, 3],
            }],
        };
        let status = tonic::Status::with_details(
            tonic::Code::Unavailable,
            "down",
            status_proto.encode_to_vec().into(),
        );
        assert_eq!(Status::from_tonic(&status).retry_delay(), None);
    }
}
