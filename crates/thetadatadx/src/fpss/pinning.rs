//! SPKI certificate pinning for FPSS TLS connections.
//!
//! # Threat model
//!
//! `ThetaData`'s FPSS servers present X.509 certificates whose `notAfter`
//! expired on `Jan 12 23:20:23 2024 GMT`, so full `webpki` trust-chain +
//! validity checks cannot succeed. Historically the client worked around
//! that by disabling all certificate verification -- which converted a
//! single-vendor expiry problem into a global **MITM + credential harvest**
//! problem: any attacker on the path to `nj-*.thetadata.us:20000` could
//! present any cert and receive the user's email + password in the
//! `CREDENTIALS` frame that immediately follows the handshake.
//!
//! # Mitigation
//!
//! This module implements `rustls`'s `ServerCertVerifier` trait with a
//! **SubjectPublicKeyInfo (SPKI) pin** against the known public key of the
//! `ThetaData` FPSS endpoints. The pin survives cert renewal as long as
//! `ThetaData` keeps the same keypair; a keypair rotation will break the
//! pin on purpose -- the ceremony of re-capturing [`FPSS_SPKI_SHA256`] is
//! exactly the human-in-the-loop checkpoint we want before we trust a new
//! key.
//!
//! Verification has three steps, all of which must pass:
//!
//! 1. **SPKI pin** -- SHA-256 of the presented leaf's `SubjectPublicKeyInfo`
//!    must constant-time-equal [`FPSS_SPKI_SHA256`]. This authenticates
//!    the server.
//! 2. **Hostname allowlist** -- the SNI we connected with must be one of
//!    the known `ThetaData` FPSS hostnames ([`ALLOWED_FPSS_HOSTS`]).
//!    Catches configuration mistakes where we connect to an unexpected
//!    host that happens to share the pinned keypair.
//! 3. **TLS signature verification** -- the TLS 1.2 / 1.3 handshake
//!    signature is verified via `rustls`'s built-in `webpki` routines.
//!    This ensures integrity of the current handshake (the server actually
//!    holds the private key for the pinned public key).
//!
//! We deliberately do **not** call `webpki::verify_server_cert` with a
//! trust anchor: that would reintroduce the expiry problem. SPKI pinning
//! subsumes identity validation at the cost of vendor-lock-in to a
//! specific keypair -- an acceptable trade for a closed protocol like
//! FPSS where the set of legitimate servers is fixed.
//!
//! # Capture procedure
//!
//! The pinned digest in [`FPSS_SPKI_SHA256`] was captured on
//! `2026-04-20` from `nj-a.thetadata.us:20000` via:
//!
//! ```text
//! openssl s_client -connect nj-a.thetadata.us:20000 \
//!     -servername nj-a.thetadata.us < /dev/null \
//!   | openssl x509 -pubkey -noout \
//!   | openssl pkey -pubin -outform DER \
//!   | openssl dgst -sha256 -binary \
//!   | xxd -p
//! ```
//!
//! The same SPKI is served by `nj-a:20000`, `nj-a:20001`, `nj-b:20000`,
//! `nj-b:20001` (production), `nj-a:20200` (dev), and `nj-a:20100`
//! (stage), so one constant covers every environment.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// SHA-256 of the `SubjectPublicKeyInfo` presented by every `ThetaData`
/// FPSS endpoint (prod `nj-a:20000`, `nj-a:20001`, `nj-b:20000`, `nj-b:20001`;
/// dev `nj-a:20200`; stage `nj-a:20100`).
///
/// Captured 2026-04-20 via the `openssl` pipeline documented in the module
/// header. SPKI (not whole-cert) so the pin survives `ThetaData`'s cert
/// renewal as long as they keep the same keypair. If they rotate the key,
/// this constant **must** be re-captured after out-of-band confirmation --
/// which is precisely the ceremony we want.
pub(crate) const FPSS_SPKI_SHA256: [u8; 32] = [
    0x21, 0x5f, 0x28, 0xe0, 0x62, 0x08, 0xc4, 0x89, 0xcb, 0xb5, 0x18, 0x4e, 0xbd, 0x94, 0x84, 0x2c,
    0xf4, 0x10, 0xae, 0x33, 0xea, 0xaa, 0xe1, 0xcf, 0xfa, 0x84, 0xdb, 0x6e, 0x92, 0x89, 0x49, 0x96,
];

/// Hostnames we are willing to connect to for FPSS.
///
/// Even with SPKI pinning we keep an explicit hostname allowlist: if a
/// config typo or DNS shenanigan points us at an unexpected host that
/// happens to share the pinned keypair, we still fail closed.
pub(crate) const ALLOWED_FPSS_HOSTS: &[&str] = &["nj-a.thetadata.us", "nj-b.thetadata.us"];

/// `rustls` server-cert verifier that enforces the FPSS SPKI pin.
///
/// See the module-level docs for the verification strategy.
#[derive(Debug)]
pub(crate) struct PinnedVerifier {
    /// The signature-verification algorithms provided by the active
    /// crypto provider. Used to verify the TLS 1.2 / 1.3 handshake
    /// signature after the SPKI pin matches.
    supported_algs: WebPkiSupportedAlgorithms,
}

impl PinnedVerifier {
    /// Build a verifier pinned to the FPSS public key, using the
    /// signature algorithms from the `ring` crypto provider.
    ///
    /// `ring` is the provider installed by [`super::connection::ensure_rustls_crypto_provider`],
    /// so the algorithms here will match what `rustls` actually uses on the
    /// wire during the handshake.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            supported_algs: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
        })
    }
}

impl ServerCertVerifier for PinnedVerifier {
    /// Verify the leaf certificate by:
    /// 1. extracting its `SubjectPublicKeyInfo` DER,
    /// 2. SHA-256-hashing that DER,
    /// 3. constant-time-comparing against [`FPSS_SPKI_SHA256`],
    /// 4. asserting the `server_name` is an allowed FPSS hostname.
    ///
    /// Intermediate certificates, the OCSP response, and `now` are
    /// deliberately ignored: the pin is on the leaf's public key alone,
    /// and the cert's expiry has been a known issue since January 2024.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        // Step 1: hostname allowlist. Do this first so a misconfigured
        // host never reaches the key-material code path.
        let hostname = server_name.to_str();
        if !ALLOWED_FPSS_HOSTS.iter().any(|h| *h == hostname.as_ref()) {
            return Err(RustlsError::InvalidCertificate(
                CertificateError::NotValidForName,
            ));
        }

        // Step 2: parse the leaf cert as DER, fail closed on any error.
        // Use x509_parser, a vetted parser for RFC 5280 certificates.
        let (_, parsed) = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| RustlsError::InvalidCertificate(CertificateError::BadEncoding))?;

        // Step 3: SHA-256 over the exact DER bytes of the SPKI SEQUENCE.
        // `subject_pki.raw` is the full ASN.1 `SubjectPublicKeyInfo`
        // structure (SEQUENCE { algorithm, subjectPublicKey }), which is
        // what `openssl pkey -pubin -outform DER` emits and what every
        // SPKI-pin tool hashes.
        let spki_digest = Sha256::digest(parsed.tbs_certificate.subject_pki.raw);

        // Step 4: constant-time equality against the pinned digest.
        // `ct_eq` returns a `subtle::Choice`; `.into()` converts to bool
        // without short-circuiting, preventing a timing side channel on
        // the prefix of the digest.
        let matches: bool = spki_digest.as_slice().ct_eq(&FPSS_SPKI_SHA256).into();
        if !matches {
            return Err(RustlsError::InvalidCertificate(
                CertificateError::NotValidForName,
            ));
        }

        Ok(ServerCertVerified::assertion())
    }

    /// Verify the TLS 1.2 handshake signature using `webpki` against the
    /// leaf cert's public key. The SPKI pin in `verify_server_cert`
    /// already confirmed *which* public key we accept; this confirms the
    /// server actually controls the corresponding private key for *this*
    /// handshake, closing the replay / impersonation gap.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    /// TLS 1.3 analogue of `verify_tls12_signature`.
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    /// Schemes we are willing to verify in [`verify_tls12_signature`] /
    /// [`verify_tls13_signature`]. Delegates to the provider's list so we
    /// never advertise a scheme the provider cannot actually check.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leaf certificate DER captured on 2026-04-20 from
    /// `nj-a.thetadata.us:20000`. Used by the positive-path test below.
    /// If `ThetaData` rotates their keypair, this fixture must be
    /// re-captured together with [`FPSS_SPKI_SHA256`].
    const THETADATA_FPSS_LEAF_DER: &[u8] =
        include_bytes!("../../tests/fixtures/thetadata_fpss_leaf.der");

    /// Install the `ring` crypto provider for the test process. Needed
    /// because `PinnedVerifier::new` reaches into `ring`'s
    /// `signature_verification_algorithms`, which works without the
    /// provider being *installed*, but any test that also constructs a
    /// `ClientConfig` will otherwise panic.
    fn install_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    fn server_name(host: &'static str) -> ServerName<'static> {
        ServerName::try_from(host).expect("valid DNS name")
    }

    fn now() -> UnixTime {
        // The FPSS cert expired in Jan 2024, and we deliberately don't
        // consult `now` in verification. Passing a fixed post-expiry
        // timestamp demonstrates that the verifier does not care about
        // cert validity windows.
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_713_600_000))
    }

    #[test]
    fn pin_matches_captured_thetadata_leaf() {
        install_provider();
        let verifier = PinnedVerifier::new();
        let leaf = CertificateDer::from(THETADATA_FPSS_LEAF_DER);

        let result =
            verifier.verify_server_cert(&leaf, &[], &server_name("nj-a.thetadata.us"), &[], now());
        assert!(
            result.is_ok(),
            "live ThetaData FPSS cert must match the captured SPKI pin: {result:?}"
        );
    }

    #[test]
    fn pin_matches_on_sibling_fpss_host() {
        install_provider();
        let verifier = PinnedVerifier::new();
        let leaf = CertificateDer::from(THETADATA_FPSS_LEAF_DER);

        // Same cert served from nj-b -- the SPKI pin + allowlist should
        // still pass.
        let result =
            verifier.verify_server_cert(&leaf, &[], &server_name("nj-b.thetadata.us"), &[], now());
        assert!(result.is_ok(), "nj-b.thetadata.us must pass: {result:?}");
    }

    #[test]
    fn unknown_hostname_is_rejected_even_with_valid_pin() {
        install_provider();
        let verifier = PinnedVerifier::new();
        let leaf = CertificateDer::from(THETADATA_FPSS_LEAF_DER);

        // Same cert, wrong hostname -- must fail. This is the defense
        // against a misconfigured hosts list that accidentally points at
        // an attacker-controlled host sharing the keypair.
        let result =
            verifier.verify_server_cert(&leaf, &[], &server_name("evil.example.com"), &[], now());
        assert!(matches!(
            result,
            Err(RustlsError::InvalidCertificate(
                CertificateError::NotValidForName
            ))
        ));
    }

    #[test]
    fn malformed_certificate_is_rejected() {
        install_provider();
        let verifier = PinnedVerifier::new();
        // Obvious garbage -- not a valid X.509 DER.
        let leaf = CertificateDer::from(&[0u8; 32][..]);

        let result =
            verifier.verify_server_cert(&leaf, &[], &server_name("nj-a.thetadata.us"), &[], now());
        assert!(matches!(
            result,
            Err(RustlsError::InvalidCertificate(
                CertificateError::BadEncoding
            ))
        ));
    }

    #[test]
    fn pinned_digest_matches_openssl_output() {
        // Guards against a typo in `FPSS_SPKI_SHA256`: recompute the
        // digest from the captured leaf and assert byte-for-byte
        // equality with the hard-coded pin. If this fails, the pin has
        // drifted from the fixture -- audit which one is correct.
        let (_, parsed) = x509_parser::parse_x509_certificate(THETADATA_FPSS_LEAF_DER)
            .expect("fixture parses as X.509");
        let digest = Sha256::digest(parsed.tbs_certificate.subject_pki.raw);
        assert_eq!(digest.as_slice(), &FPSS_SPKI_SHA256);
    }

    #[test]
    fn allowed_hosts_cover_every_fpss_environment() {
        // Prod, dev, and stage all use either nj-a or nj-b. Keep this
        // test in sync with `DirectConfig::{production,dev,stage}` so we
        // notice if a new environment is added that needs an allowlist
        // entry.
        for host in ["nj-a.thetadata.us", "nj-b.thetadata.us"] {
            assert!(
                ALLOWED_FPSS_HOSTS.contains(&host),
                "{host} must be in ALLOWED_FPSS_HOSTS"
            );
        }
    }
}
