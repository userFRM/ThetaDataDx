//! ThetaData HTTP/gRPC error code mapping.
//!
//! The MDDS server returns numeric error codes in HTTP status and gRPC
//! metadata. This module maps those codes to human-readable names and
//! descriptions.

/// A ThetaData error code with its HTTP status, short name, and description.
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
pub fn error_from_http_code(code: u16) -> Option<&'static ThetaDataError> {
    ERRORS.iter().find(|e| e.http_code == code)
}

/// Extract an `http_status_code` from gRPC metadata text and look it up.
///
/// The metadata string is expected to contain `http_status_code=NNN` or just
/// the numeric code itself. Returns `None` if no valid code is found or the
/// code is not in the known error table.
pub fn error_from_grpc_metadata(metadata: &str) -> Option<&'static ThetaDataError> {
    // Try "http_status_code=NNN" first.
    if let Some(pos) = metadata.find("http_status_code=") {
        let after = &metadata[pos + "http_status_code=".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num_str.parse::<u16>() {
            return error_from_http_code(code);
        }
    }
    // Fallback: try parsing the whole string as a number.
    if let Ok(code) = metadata.trim().parse::<u16>() {
        return error_from_http_code(code);
    }
    None
}

/// Human-readable error name for an HTTP status code, or `"UNKNOWN"`.
#[inline]
pub fn error_name(code: u16) -> &'static str {
    error_from_http_code(code).map_or("UNKNOWN", |e| e.name)
}

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

    #[test]
    fn grpc_metadata_key_value() {
        let meta = "http_status_code=473";
        let err = error_from_grpc_metadata(meta).unwrap();
        assert_eq!(err.name, "INVALID_PARAMS");
    }

    #[test]
    fn grpc_metadata_embedded() {
        let meta = "some_prefix http_status_code=476 some_suffix";
        let err = error_from_grpc_metadata(meta).unwrap();
        assert_eq!(err.name, "WRONG_IP");
    }

    #[test]
    fn grpc_metadata_bare_number() {
        let err = error_from_grpc_metadata("471").unwrap();
        assert_eq!(err.name, "PERMISSION");
    }

    #[test]
    fn grpc_metadata_unknown() {
        assert!(error_from_grpc_metadata("garbage").is_none());
        assert!(error_from_grpc_metadata("http_status_code=999").is_none());
    }

    #[test]
    fn error_name_helper() {
        assert_eq!(error_name(429), "OS_LIMIT");
        assert_eq!(error_name(999), "UNKNOWN");
    }
}
