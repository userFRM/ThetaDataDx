//! Server identity verification for the MDDS legacy port.
//!
//! The MDDS endpoints (`nj-{a,b}.thetadata.us:12000-12001`) present a TLS
//! certificate whose chain has been expired since `2024-01-12`. Standard
//! webpki validation fails. The vendor terminal works around this by trusting
//! a single bundled cert from `client.jks`. We work around it by pinning the
//! SubjectPublicKeyInfo (SPKI) of the leaf certificate — the same approach
//! the FPSS module already uses (see [`crate::fpss::pinning`]). Live
//! observation on `2026-04-29` confirms the same SPKI is served by both the
//! FPSS port (20000) and the MDDS port (12000) — ThetaData runs one keypair
//! across the two backends. We therefore reuse the FPSS pin constant rather
//! than maintaining a parallel one.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use x509_parser::prelude::*;

use crate::fpss::pinning::FPSS_SPKI_SHA256;

/// Hostnames we will connect to for MDDS legacy.
pub(crate) const ALLOWED_MDDS_HOSTS: &[&str] = &["nj-a.thetadata.us", "nj-b.thetadata.us"];

/// Production MDDS legacy ports for the `nj-{a,b}` region.
pub(crate) const MDDS_PORTS: &[u16] = &[12000, 12001];

#[derive(Debug)]
pub(crate) struct MddsSpkiVerifier {
    algs: WebPkiSupportedAlgorithms,
}

impl MddsSpkiVerifier {
    pub(crate) fn new() -> Arc<Self> {
        // Install the ring provider on first use so callers don't have to
        // remember to do it themselves. Subsequent installs are a no-op.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let provider = rustls::crypto::CryptoProvider::get_default()
            .expect("rustls ring provider just installed");
        Arc::new(Self {
            algs: provider.signature_verification_algorithms,
        })
    }
}

impl ServerCertVerifier for MddsSpkiVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Hostname allowlist — even with SPKI pinning, refuse unknown SNIs.
        let host = match server_name {
            ServerName::DnsName(d) => d.as_ref().to_string(),
            other => return Err(rustls::Error::General(format!("unexpected SNI: {other:?}"))),
        };
        if !ALLOWED_MDDS_HOSTS.contains(&host.as_str()) {
            return Err(rustls::Error::General(format!(
                "MDDS host {host} not in allowlist"
            )));
        }

        // Parse leaf and extract SPKI bytes.
        let (_, parsed) = X509Certificate::from_der(end_entity.as_ref())
            .map_err(|e| rustls::Error::General(format!("MDDS leaf cert parse failed: {e}")))?;
        let spki_der = parsed.tbs_certificate.subject_pki.raw;
        let digest: [u8; 32] = Sha256::digest(spki_der).into();

        if digest.ct_eq(&FPSS_SPKI_SHA256).unwrap_u8() != 1 {
            return Err(rustls::Error::General(
                "MDDS server SPKI does not match pinned ThetaData keypair".to_string(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}
