//! How requests reach servers: through a proxy or not, with which client certificates and extra trusted CAs, and with
//! which cookie jar. These are settings of the machine, not of a request; see `crate::settings`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use url::Url;

/// Cookies the server set, sent back on later requests to matching URLs.
pub type CookieJar = Arc<Mutex<cookie_store::CookieStore>>;

#[derive(Debug, Default)]
pub struct Network {
    pub proxy: Option<Proxy>,
    /// Tried in order; the first whose host matches is used.
    pub client_certificates: Vec<ClientCertificate>,
    /// Trusted on top of the operating system's CAs, such as a company or development CA.
    pub ca_certificates: Vec<CertificateDer<'static>>,
    pub cookies: Option<CookieJar>,
    /// Domains whose cookies scripts may read and change with `pm.cookies.jar()`.
    pub script_cookie_domains: Vec<String>,
}

/// A proxy for HTTP and one for HTTPS (often the same), and hosts that bypass them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Proxy {
    pub http: Option<Url>,
    pub https: Option<Url>,
    /// As in `NO_PROXY`: `example.com` (and its subdomains), `.example.com`, `*`, `10.0.0.1`, `localhost:3000`.
    pub bypass: Vec<String>,
}

impl Proxy {
    /// The proxy settings in `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` (either case), as curl reads them.
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| {
            std::env::var(name.to_lowercase())
                .or_else(|_| std::env::var(name))
                .ok()
                .filter(|v| !v.trim().is_empty())
        };
        let parse = |v: Option<String>| v.and_then(|v| parse_proxy_url(&v).ok());
        let all = parse(var("ALL_PROXY"));
        let proxy = Proxy {
            http: parse(var("HTTP_PROXY")).or_else(|| all.clone()),
            https: parse(var("HTTPS_PROXY")).or(all),
            bypass: var("NO_PROXY")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        };
        (proxy.http.is_some() || proxy.https.is_some()).then_some(proxy)
    }

    /// The proxy to use for `url`, if any.
    pub fn for_url(&self, url: &Url) -> Option<&Url> {
        let proxy = match url.scheme() {
            "https" | "wss" => self.https.as_ref(),
            _ => self.http.as_ref(),
        }?;
        let host = url
            .host_str()?
            .trim_matches(['[', ']'])
            .to_ascii_lowercase();
        let port = url.port_or_known_default();
        let bypassed = self.bypass.iter().any(|rule| {
            let rule = rule.trim().to_ascii_lowercase();
            if rule == "*" {
                return true;
            }
            let (rule_host, rule_port) = match rule.rsplit_once(':') {
                Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !h.contains(':') => {
                    (h, p.parse().ok())
                }
                _ => (rule.as_str(), None),
            };
            if rule_port.is_some() && rule_port != port {
                return false;
            }
            let rule_host = rule_host.trim_start_matches("*.").trim_start_matches('.');
            host == rule_host || host.ends_with(&format!(".{rule_host}"))
        });
        (!bypassed).then_some(proxy)
    }
}

/// Reads a proxy URL; `proxy.local:8080` means `http://proxy.local:8080`.
pub fn parse_proxy_url(text: &str) -> Result<Url, String> {
    let text = text.trim();
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else {
        format!("http://{text}")
    };
    let url = Url::parse(&with_scheme).map_err(|e| format!("proxy \"{text}\": {e}"))?;
    match url.scheme() {
        "http" => Ok(url),
        other => Err(format!(
            "proxy \"{text}\": {other}:// proxies are not supported yet; use an http:// proxy (it can still carry HTTPS)"
        )),
    }
}

/// A certificate to present to servers matching `host`.
#[derive(Debug)]
pub struct ClientCertificate {
    /// A host name, `*.example.com`, or either with `:port`.
    pub host: String,
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl Clone for ClientCertificate {
    fn clone(&self) -> Self {
        Self {
            host: self.host.clone(),
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl ClientCertificate {
    /// From PEM files: the certificate (and any intermediates) and its private key.
    pub fn from_pem(host: &str, cert: &Path, key: &Path) -> Result<Self, String> {
        let chain = CertificateDer::pem_file_iter(cert)
            .map_err(|e| format!("{}: {e}", cert.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: {e}", cert.display()))?;
        if chain.is_empty() {
            return Err(format!("{}: no certificate in the file", cert.display()));
        }
        let key =
            PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
        Ok(Self {
            host: host.to_string(),
            chain,
            key,
        })
    }

    /// From a PKCS#12 file (`.pfx` or `.p12`) and its passphrase.
    pub fn from_pfx(host: &str, pfx: &Path, passphrase: &str) -> Result<Self, String> {
        let data = std::fs::read(pfx).map_err(|e| format!("{}: {e}", pfx.display()))?;
        let store = p12_keystore::KeyStore::from_pkcs12(
            &data,
            passphrase,
            p12_keystore::Pkcs12ImportPolicy::Relaxed,
        )
        .map_err(|e| format!("{}: {e} (is the passphrase right?)", pfx.display()))?;
        let (_, chain) = store
            .private_key_chain()
            .ok_or_else(|| format!("{}: no private key in the file", pfx.display()))?;
        let certs: Vec<CertificateDer<'static>> = chain
            .certs()
            .iter()
            .map(|c| CertificateDer::from(c.as_der().to_vec()))
            .collect();
        if certs.is_empty() {
            return Err(format!(
                "{}: no certificate for the private key",
                pfx.display()
            ));
        }
        let key = PrivateKeyDer::try_from(chain.key().as_der().to_vec())
            .map_err(|e| format!("{}: {e}", pfx.display()))?;
        Ok(Self {
            host: host.to_string(),
            chain: certs,
            key,
        })
    }

    pub fn matches(&self, host: &str, port: Option<u16>) -> bool {
        let pattern = self.host.trim().to_ascii_lowercase();
        let (pattern, pattern_port) = match pattern.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
                (h.to_string(), p.parse::<u16>().ok())
            }
            _ => (pattern.clone(), None),
        };
        if pattern_port.is_some() && pattern_port != port {
            return false;
        }
        let host = host.to_ascii_lowercase();
        match pattern.strip_prefix("*.") {
            Some(base) => host.ends_with(&format!(".{base}")),
            None => host == pattern,
        }
    }
}

/// Reads CA certificates from a PEM file.
pub fn read_ca_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{}: no certificate in the file", path.display()));
    }
    Ok(certs)
}

impl Network {
    pub fn client_certificate(&self, host: &str, port: Option<u16>) -> Option<&ClientCertificate> {
        self.client_certificates
            .iter()
            .find(|c| c.matches(host, port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_rules() {
        let proxy = Proxy {
            http: Some(Url::parse("http://proxy:3128").unwrap()),
            https: Some(Url::parse("http://proxy:3128").unwrap()),
            bypass: vec![
                "localhost".into(),
                ".internal.example".into(),
                "10.0.0.5".into(),
                "api.test:8443".into(),
            ],
        };
        let url = |u: &str| Url::parse(u).unwrap();
        assert!(proxy.for_url(&url("https://example.com/")).is_some());
        assert!(proxy.for_url(&url("http://localhost:3000/")).is_none());
        assert!(proxy.for_url(&url("https://a.internal.example/")).is_none());
        assert!(proxy.for_url(&url("http://10.0.0.5/")).is_none());
        assert!(proxy.for_url(&url("https://api.test:8443/")).is_none());
        assert!(proxy.for_url(&url("https://api.test/")).is_some());
        assert!(parse_proxy_url("socks5://x:1080").is_err());
        assert_eq!(
            parse_proxy_url("proxy.local:8080").unwrap().as_str(),
            "http://proxy.local:8080/"
        );
    }

    #[test]
    fn certificate_hosts() {
        let cert = |host: &str| ClientCertificate {
            host: host.into(),
            chain: vec![],
            key: PrivateKeyDer::try_from(vec![0u8; 0])
                .unwrap_or_else(|_| PrivateKeyDer::Pkcs8(vec![].into())),
        };
        assert!(cert("api.example.com").matches("API.example.com", Some(443)));
        assert!(!cert("api.example.com").matches("example.com", Some(443)));
        assert!(cert("*.example.com").matches("a.example.com", Some(443)));
        assert!(!cert("*.example.com:8443").matches("a.example.com", Some(443)));
        assert!(cert("*.example.com:8443").matches("a.example.com", Some(8443)));
    }
}
