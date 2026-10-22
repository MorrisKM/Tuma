//! Settings of this machine, not of a workspace: proxy, client certificates, extra CA certificates and cookies. They
//! live in Tuma's data folder (`settings.json`, shared by the app, the CLI and the MCP server), because paths and
//! proxies differ between machines and don't belong in git. Passwords and passphrases are in the OS keychain.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::http::network::{self, ClientCertificate, CookieJar, Network, Proxy};
use crate::secrets::SecretStore;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MachineSettings {
    pub proxy: ProxySettings,
    pub client_certificates: Vec<CertificateSetting>,
    /// PEM files with CA certificates to trust on top of the operating system's.
    pub ca_certificates: Vec<PathBuf>,
    pub cookies: CookieSettings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`, as curl reads them.
    #[default]
    System,
    None,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxySettings {
    pub mode: ProxyMode,
    /// For `custom`: `host:port` or `http://host:port`.
    pub http: String,
    /// For `custom`: the proxy for HTTPS; empty means the same as `http`.
    pub https: String,
    /// Hosts that skip the proxy, as in `NO_PROXY`.
    pub bypass: Vec<String>,
    /// The proxy's user name, if it needs one. The password is in the keychain.
    pub username: String,
}

/// A client certificate for a host: PEM certificate and key files, or a PKCS#12 file (whose passphrase is in the
/// keychain).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CertificateSetting {
    /// `api.example.com`, `*.example.com`, optionally with `:port`.
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pfx: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CookieSettings {
    /// Keep cookies servers set and send them back, per workspace.
    pub enabled: bool,
    /// Domains whose cookies scripts may read and change with `pm.cookies.jar()`.
    pub script_domains: Vec<String>,
}

impl Default for CookieSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            script_domains: vec![],
        }
    }
}

/// Keychain names for this machine's secrets.
pub const PROXY_PASSWORD: &str = "machine/proxy-password";
pub fn certificate_passphrase(host: &str) -> String {
    format!("machine/certificate/{host}")
}

fn file() -> Option<PathBuf> {
    crate::history::data_dir().map(|d| d.join("settings.json"))
}

impl MachineSettings {
    /// The saved settings, or the defaults when there are none or they can't be read.
    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = file().ok_or("no data folder on this machine")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The proxy these settings select, if any.
    pub fn proxy(&self, secrets: &dyn SecretStore) -> Result<Option<Proxy>, String> {
        match self.proxy.mode {
            ProxyMode::None => Ok(None),
            ProxyMode::System => Ok(Proxy::from_env()),
            ProxyMode::Custom => {
                if self.proxy.http.trim().is_empty() {
                    return Ok(None);
                }
                let mut http = network::parse_proxy_url(&self.proxy.http)?;
                let mut https = if self.proxy.https.trim().is_empty() {
                    http.clone()
                } else {
                    network::parse_proxy_url(&self.proxy.https)?
                };
                if !self.proxy.username.is_empty() {
                    let password = secrets
                        .get(PROXY_PASSWORD)
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    for url in [&mut http, &mut https] {
                        let _ = url.set_username(&self.proxy.username);
                        let _ = url.set_password(Some(&password));
                    }
                }
                Ok(Some(Proxy {
                    http: Some(http),
                    https: Some(https),
                    bypass: self.proxy.bypass.clone(),
                }))
            }
        }
    }

    /// Everything a request needs from these settings, with the cookie jar of `workspace` (by its id) when cookies
    /// are on. Fails when a certificate file can't be read.
    pub fn network(
        &self,
        secrets: &dyn SecretStore,
        workspace: Option<&str>,
    ) -> Result<Network, String> {
        let mut client_certificates = Vec::new();
        for c in &self.client_certificates {
            if c.host.trim().is_empty() {
                continue;
            }
            let cert = match (&c.pfx, &c.cert, &c.key) {
                (Some(pfx), _, _) => {
                    let passphrase = secrets
                        .get(&certificate_passphrase(&c.host))
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    ClientCertificate::from_pfx(&c.host, pfx, &passphrase)?
                }
                (None, Some(cert), Some(key)) => ClientCertificate::from_pem(&c.host, cert, key)?,
                _ => {
                    return Err(format!(
                        "the client certificate for {} needs a PFX file, or a CRT and a KEY file",
                        c.host
                    ));
                }
            };
            client_certificates.push(cert);
        }
        let mut ca_certificates = Vec::new();
        for path in &self.ca_certificates {
            ca_certificates.extend(network::read_ca_certificates(path)?);
        }
        Ok(Network {
            proxy: self.proxy(secrets)?,
            client_certificates,
            ca_certificates,
            cookies: match workspace {
                Some(id) if self.cookies.enabled => Some(load_cookies(id)),
                _ => None,
            },
            script_cookie_domains: self.cookies.script_domains.clone(),
        })
    }
}

fn cookies_file(workspace: &str) -> Option<PathBuf> {
    let id: String = workspace
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    crate::history::data_dir().map(|d| d.join("cookies").join(format!("{id}.json")))
}

/// A workspace's cookie jar: session cookies too, so a login in one `tuma` run works in the next.
pub fn load_cookies(workspace: &str) -> CookieJar {
    let store = cookies_file(workspace)
        .and_then(|f| std::fs::File::open(f).ok())
        .and_then(|f| cookie_store::serde::json::load_all(std::io::BufReader::new(f)).ok())
        .unwrap_or_default();
    Arc::new(Mutex::new(store))
}

pub fn save_cookies(workspace: &str, jar: &CookieJar) -> Result<(), String> {
    let path = cookies_file(workspace).ok_or("no data folder on this machine")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut out = Vec::new();
    cookie_store::serde::json::save_incl_expired_and_nonpersistent(&jar.lock().unwrap(), &mut out)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// A cookie as the cookie manager and scripts show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieView {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Unix time in seconds; absent for a session cookie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
}

/// The cookies in a jar, by domain.
pub fn list_cookies(jar: &CookieJar) -> BTreeMap<String, Vec<CookieView>> {
    let mut out: BTreeMap<String, Vec<CookieView>> = BTreeMap::new();
    for c in jar.lock().unwrap().iter_unexpired() {
        // Host-only cookies (no Domain attribute) are stored under their host.
        let domain = String::from(&c.domain).trim_start_matches('.').to_string();
        out.entry(domain.clone()).or_default().push(CookieView {
            domain,
            path: String::from(&c.path),
            name: c.name().to_string(),
            value: c.value().to_string(),
            expires: c.expires_datetime().map(|t| t.unix_timestamp()),
            secure: c.secure().unwrap_or(false),
            http_only: c.http_only().unwrap_or(false),
        });
    }
    out
}

/// Removes one cookie, every cookie of a domain (`name` `None`), or everything (`domain` `None`).
pub fn remove_cookies(
    jar: &CookieJar,
    domain: Option<&str>,
    path: Option<&str>,
    name: Option<&str>,
) {
    let mut store = jar.lock().unwrap();
    let Some(domain) = domain else {
        store.clear();
        return;
    };
    let matching: Vec<(String, String, String)> = store
        .iter_any()
        .map(|c| {
            (
                String::from(&c.domain),
                String::from(&c.path),
                c.name().to_string(),
            )
        })
        .filter(|(d, p, n)| {
            d.trim_start_matches('.') == domain
                && name.is_none_or(|name| n == name)
                && path.is_none_or(|path| p == path)
        })
        .collect();
    for (d, p, n) in matching {
        store.remove(&d, &p, &n);
    }
}

/// The network for requests in `workspace`, from this machine's settings: what the CLI, the MCP server and the app
/// use. Save the cookies afterwards with [`keep_cookies`].
pub fn network_for(
    workspace: Option<&crate::workspace::Workspace>,
    secrets: &dyn SecretStore,
) -> Result<Arc<Network>, String> {
    Ok(Arc::new(
        MachineSettings::load().network(secrets, workspace.map(|w| w.id()))?,
    ))
}

/// Saves the cookie jar of a network made by [`network_for`].
pub fn keep_cookies(
    network: &Network,
    workspace: Option<&crate::workspace::Workspace>,
) -> Result<(), String> {
    match (&network.cookies, workspace) {
        (Some(jar), Some(ws)) => save_cookies(ws.id(), jar),
        _ => Ok(()),
    }
}
