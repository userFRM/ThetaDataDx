//! Encoding-layer errors for `ThetaData` Binary Encoding, plus the
//! `ThetaData` HTTP error-code lookup table consumed by the networking
//! crate when folding `tonic::Status` into a `thetadatadx::Error`.

use thiserror::Error;

/// Encoding-layer errors for `ThetaData` Binary Encoding.
#[derive(Error, Debug)]
pub enum Error {
    /// FIT nibble decoding failure (malformed input, unexpected terminator).
    #[error("FIT decode error: {0}")]
    Decode(String),

    /// FIE nibble encoding failure (invalid character).
    #[error("FIE encode error: {0}")]
    Encode(String),

    /// Value conversion error (e.g., enum from invalid discriminant).
    #[error("conversion error: {0}")]
    Conversion(String),

    /// Configuration / input validation error (e.g., unrecognised `right`
    /// string supplied to [`crate::right::parse_right`]).
    #[error("Configuration error: {0}")]
    Config(String),

    /// I/O error during read/write operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience Result type.
pub type Result<T> = std::result::Result<T, Error>;

// ─── ThetaData HTTP error-code lookup ───────────────────────────────────

/// A `ThetaData` error code with its HTTP status, short name, and description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThetaDataError {
    pub http_code: u16,
    pub name: &'static str,
    pub description: &'static str,
}

const ERRORS: &[ThetaDataError] = &[
    ThetaDataError {
        http_code: 200,
        name: "OK",
        description: "Request completed successfully.",
    },
    ThetaDataError {
        http_code: 404,
        name: "NO_IMPL",
        description: "Endpoint or feature is not implemented.",
    },
    ThetaDataError {
        http_code: 429,
        name: "OS_LIMIT",
        description: "Rate limit exceeded for the current subscription tier.",
    },
    ThetaDataError {
        http_code: 470,
        name: "GENERAL",
        description: "General server-side error.",
    },
    ThetaDataError {
        http_code: 471,
        name: "PERMISSION",
        description: "Insufficient permissions for the requested data.",
    },
    ThetaDataError {
        http_code: 472,
        name: "NO_DATA",
        description: "No data available for the requested parameters.",
    },
    ThetaDataError {
        http_code: 473,
        name: "INVALID_PARAMS",
        description: "One or more request parameters are invalid.",
    },
    ThetaDataError {
        http_code: 474,
        name: "DISCONNECTED",
        description: "Client is disconnected from the server.",
    },
    ThetaDataError {
        http_code: 475,
        name: "TERMINAL_PARSE",
        description: "Server failed to parse the terminal request.",
    },
    ThetaDataError {
        http_code: 476,
        name: "WRONG_IP",
        description: "Request originated from an unauthorized IP address.",
    },
    ThetaDataError {
        http_code: 477,
        name: "NO_PAGE_FOUND",
        description: "The requested page was not found.",
    },
    ThetaDataError {
        http_code: 478,
        name: "INVALID_SESSION_ID",
        description: "The session ID is invalid or expired.",
    },
    ThetaDataError {
        http_code: 571,
        name: "SERVER_STARTING",
        description: "Server is still starting up; retry shortly.",
    },
    ThetaDataError {
        http_code: 572,
        name: "UNCAUGHT_ERROR",
        description: "An uncaught server-side error occurred.",
    },
];

/// Look up a `ThetaDataError` by its HTTP status code.
#[inline]
#[must_use]
pub fn error_from_http_code(code: u16) -> Option<&'static ThetaDataError> {
    ERRORS.iter().find(|e| e.http_code == code)
}

/// Metadata key carrying the ThetaData HTTP status in gRPC responses.
pub const HTTP_STATUS_CODE_KEY: &str = "http_status_code";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes() {
        assert_eq!(error_from_http_code(200).unwrap().name, "OK");
        assert_eq!(error_from_http_code(472).unwrap().name, "NO_DATA");
        assert_eq!(error_from_http_code(571).unwrap().name, "SERVER_STARTING");
        assert_eq!(error_from_http_code(572).unwrap().name, "UNCAUGHT_ERROR");
    }

    #[test]
    fn unknown_code() {
        assert!(error_from_http_code(999).is_none());
        assert!(error_from_http_code(500).is_none());
    }
}
