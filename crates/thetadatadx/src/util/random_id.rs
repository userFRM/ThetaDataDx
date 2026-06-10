//! Cryptographically random identifiers for scratch paths and transient tokens.

/// 16-byte random identifier rendered as 32 lowercase hex characters (no hyphens).
///
/// Suitable for disambiguating concurrent scratch-file paths and ephemeral
/// session tokens. Entropy is 128 bits via `rand::random` — collision
/// probability at the expected cardinality is negligible.
pub(crate) fn random_id_hex() -> String {
    let bytes: [u8; 16] = rand::random();
    let mut out = String::with_capacity(32);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Validate that `s` is a UUID string with the canonical
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` shape (8-4-4-4-12 hex groups
/// separated by hyphens; both hex cases accepted). Does **not** enforce
/// version or variant nibbles, so v1, v4, and v7 all pass.
///
/// Returns `s` unchanged on success.
pub(crate) fn validate_uuid_format(s: &str) -> Result<&str, &'static str> {
    if s.len() != 36 {
        return Err("uuid length must be 36");
    }
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return Err("uuid hyphen offset");
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return Err("uuid non-hex char");
                }
            }
        }
    }
    Ok(s)
}
