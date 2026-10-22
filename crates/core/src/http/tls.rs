//! TLS client settings. Certificates are checked against the operating system's trust store, so company and
//! locally installed CAs work without setup.

use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;

use super::Error;

/// The shared client config. Built once: loading the system trust store is slow.
pub(crate) fn config(verify: bool) -> Result<Arc<ClientConfig>, Error> {
    static VERIFIED: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    static UNVERIFIED: OnceLock<Arc<ClientConfig>> = OnceLock::new();

    if !verify {
        return Ok(UNVERIFIED.get_or_init(unverified).clone());
    }
    VERIFIED
        .get_or_init(verified)
        .clone()
        .map_err(Error::TlsConfig)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn verified() -> Result<Arc<ClientConfig>, String> {
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_platform_verifier())
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    Ok(Arc::new(with_alpn(config)))
}

fn unverified() -> Arc<ClientConfig> {
    let provider = provider();
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
        .with_no_client_auth();
    Arc::new(with_alpn(config))
}

fn with_alpn(mut config: ClientConfig) -> ClientConfig {
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}

/// Used when the user turns off certificate verification, for self-signed development servers.
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
