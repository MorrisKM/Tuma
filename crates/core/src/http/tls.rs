//! TLS client settings. Certificates are checked against the operating system's trust store, so company and
//! locally installed CAs work without setup.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;

use super::Error;
use super::network::ClientCertificate;

/// What a TLS connection needs beyond the defaults.
#[derive(Clone, Copy, Default)]
pub(crate) struct Options<'a> {
    pub verify: bool,
    pub client_certificate: Option<&'a ClientCertificate>,
    pub ca_certificates: &'a [CertificateDer<'static>],
    /// Offer only HTTP/1.1, for protocols that upgrade an HTTP/1.1 connection (WebSocket).
    pub http1_only: bool,
}

/// The client config for `options`. Configs are cached: loading the system trust store is slow.
pub(crate) fn config(options: Options) -> Result<Arc<ClientConfig>, Error> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<ClientConfig>>>> = OnceLock::new();
    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        options.verify.hash(&mut hasher);
        options.http1_only.hash(&mut hasher);
        for cert in options.ca_certificates {
            cert.as_ref().hash(&mut hasher);
        }
        if let Some(c) = options.client_certificate {
            for cert in &c.chain {
                cert.as_ref().hash(&mut hasher);
            }
            c.key.secret_der().hash(&mut hasher);
        }
        hasher.finish()
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some(config) = cache.lock().unwrap().get(&key) {
        return Ok(config.clone());
    }
    let config = Arc::new(build(options).map_err(Error::TlsConfig)?);
    cache.lock().unwrap().insert(key, config.clone());
    Ok(config)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn build(options: Options) -> Result<ClientConfig, String> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let builder = if !options.verify {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
    } else if options.ca_certificates.is_empty() {
        builder
            .with_platform_verifier()
            .map_err(|e| e.to_string())?
    } else {
        let verifier = rustls_platform_verifier::Verifier::new_with_extra_roots(
            options.ca_certificates.iter().cloned(),
            provider,
        )
        .map_err(|e| format!("could not use the CA certificates: {e}"))?;
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
    };
    let mut config = match options.client_certificate {
        Some(c) => builder
            .with_client_auth_cert(c.chain.clone(), c.key.clone_key())
            .map_err(|e| format!("the client certificate for {} can't be used: {e}", c.host))?,
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = if options.http1_only {
        vec![b"http/1.1".to_vec()]
    } else {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    };
    Ok(config)
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
