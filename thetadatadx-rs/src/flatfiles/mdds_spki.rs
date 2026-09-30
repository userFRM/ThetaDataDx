//! Server identity for the MDDS legacy port.
//!
//! The MDDS endpoints (`nj-{a,b}.thetadata.us:12000-12001`) present a TLS
//! certificate whose chain has been expired since `2024-01-12`, so standard
//! webpki chain validation fails. Trust is anchored instead by pinning the
//! SubjectPublicKeyInfo (SPKI) of the leaf certificate with the FPSS
//! verifier ([`crate::fpss::pinning::PinnedVerifier`]). The same SPKI is
//! served by both the FPSS port (20000) and the MDDS port (12000) — one
//! keypair backs both — so the verifier and its pin are shared and only the
//! host allowlist below differs.

/// Hostnames we will connect to for MDDS legacy.
pub(crate) const ALLOWED_MDDS_HOSTS: &[&str] = &["nj-a.thetadata.us", "nj-b.thetadata.us"];

/// Production MDDS legacy ports for the `nj-{a,b}` region.
pub(crate) const MDDS_PORTS: &[u16] = &[12000, 12001];
