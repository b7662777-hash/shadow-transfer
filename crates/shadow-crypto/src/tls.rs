//! TLS 1.3 transport configuration.
//!
//! Both peers present a self-signed certificate and require one from the other.
//! rustls still verifies every handshake signature and enforces TLS 1.3; the
//! certificate *chain* is deliberately not WebPKI-validated because this is a
//! private peer-to-peer protocol. Instead, the caller MUST pin the peer
//! fingerprint from `peer_certificates()` immediately after the handshake and
//! before reading or writing any application message (the session layer in
//! `shadow-transfer` does this).
//!
//! Certificate pinning is not optional: `AcceptAnyCertificate` is private to
//! this crate and is only wired into configs whose callers perform pinning.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error as RustlsError, ServerConfig,
    SignatureScheme,
};
use thiserror::Error;

use crate::Identity;

pub const ALPN_PROTOCOL: &[u8] = b"shadow-transfer/1";

#[derive(Debug, Error)]
pub enum TlsConfigError {
    #[error("invalid private key: {0}")]
    PrivateKey(String),
    #[error("invalid certificate: {0}")]
    Certificate(String),
    #[error("TLS configuration error: {0}")]
    Config(String),
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn private_key(identity: &Identity) -> Result<PrivateKeyDer<'static>, TlsConfigError> {
    let key = PrivatePkcs8KeyDer::from(identity.key_der().to_vec());
    Ok(PrivateKeyDer::Pkcs8(key))
}

#[derive(Debug)]
struct AcceptAnyCertificate {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyCertificate {
    fn new(provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        Self { provider }
    }
}

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct RequireClientCertificate {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl RequireClientCertificate {
    fn new(provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        Self { provider }
    }
}

impl ClientCertVerifier for RequireClientCertificate {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, RustlsError> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }

    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }
}

pub fn server_config(identity: &Identity) -> Result<Arc<ServerConfig>, TlsConfigError> {
    let provider = provider();
    let mut config = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsConfigError::Config(e.to_string()))?
        .with_client_cert_verifier(Arc::new(RequireClientCertificate::new(provider)))
        .with_single_cert(vec![identity.cert_der()], private_key(identity)?)
        .map_err(|e| TlsConfigError::Certificate(e.to_string()))?;
    config.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}

pub fn client_config(identity: &Identity) -> Result<Arc<ClientConfig>, TlsConfigError> {
    let provider = provider();
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsConfigError::Config(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate::new(provider)))
        .with_client_auth_cert(vec![identity.cert_der()], private_key(identity)?)
        .map_err(|e| TlsConfigError::Certificate(e.to_string()))?;
    config.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_build_with_tls13_only() {
        let identity = Identity::generate().unwrap();
        server_config(&identity).unwrap();
        client_config(&identity).unwrap();
    }
}
