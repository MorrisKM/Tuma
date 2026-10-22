//! This machine's settings (proxy, certificates, cookies) and the cookie manager. Passwords and passphrases are kept
//! in the keychain; the UI only learns whether one is set.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tuma_core::settings::{self, CookieView, MachineSettings};

use crate::workspace::{Open, secrets};

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    settings: MachineSettings,
    /// Whether a proxy password is stored.
    proxy_password_set: bool,
    /// Hosts whose certificate has a stored passphrase.
    passphrases_set: Vec<String>,
    /// What the system proxy (environment variables) is, for display.
    system_proxy: Option<String>,
}

#[tauri::command]
pub fn settings_read() -> SettingsView {
    let settings = MachineSettings::load();
    let store = secrets();
    let has = |key: &str| store.get(key).ok().flatten().is_some_and(|v| !v.is_empty());
    let system_proxy = tuma_core::http::network::Proxy::from_env().map(|p| {
        let shown = |u: &Option<url::Url>| {
            u.as_ref().map(|u| {
                u.host_str().unwrap_or_default().to_string()
                    + &u.port().map(|p| format!(":{p}")).unwrap_or_default()
            })
        };
        match (shown(&p.http), shown(&p.https)) {
            (Some(h), Some(s)) if h == s => h,
            (h, s) => format!(
                "http: {}, https: {}",
                h.unwrap_or("none".into()),
                s.unwrap_or("none".into())
            ),
        }
    });
    SettingsView {
        proxy_password_set: has(settings::PROXY_PASSWORD),
        passphrases_set: settings
            .client_certificates
            .iter()
            .filter(|c| has(&settings::certificate_passphrase(&c.host)))
            .map(|c| c.host.clone())
            .collect(),
        system_proxy,
        settings,
    }
}

/// New secrets to store with the settings. `None` leaves a secret as it is; an empty string removes it.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsSecrets {
    proxy_password: Option<String>,
    /// By certificate host.
    passphrases: BTreeMap<String, String>,
}

#[tauri::command]
pub fn settings_write(
    settings: MachineSettings,
    secrets_in: Option<SettingsSecrets>,
) -> Result<()> {
    // Fail before saving if a certificate can't be read, so a typo in a path is caught here, not at the next send.
    let new_secrets = secrets_in.unwrap_or_default();
    let store = secrets();
    let set = |key: &str, value: &str| {
        if value.is_empty() {
            store.delete(key)
        } else {
            store.set(key, value)
        }
    };
    if let Some(password) = &new_secrets.proxy_password {
        set(settings::PROXY_PASSWORD, password)?;
    }
    for (host, passphrase) in &new_secrets.passphrases {
        set(&settings::certificate_passphrase(host), passphrase)?;
    }
    settings.network(store, None)?;
    settings.save()
}

fn jar(open: &Open) -> Result<(String, tuma_core::http::network::CookieJar)> {
    let guard = open.0.lock().unwrap();
    let ws = guard.as_ref().ok_or("No workspace is open")?;
    Ok((ws.id().to_string(), settings::load_cookies(ws.id())))
}

/// The open workspace's cookies, by domain.
#[tauri::command]
pub fn cookies_list(open: tauri::State<'_, Open>) -> Result<BTreeMap<String, Vec<CookieView>>> {
    let (_, jar) = jar(&open)?;
    Ok(settings::list_cookies(&jar))
}

/// Adds a cookie for `domain` from Set-Cookie text, first removing `replacing` (the path and name of the cookie being
/// edited).
#[tauri::command]
pub fn cookies_set(
    open: tauri::State<'_, Open>,
    domain: String,
    raw: String,
    replacing: Option<(String, String)>,
) -> Result<()> {
    let (id, jar) = jar(&open)?;
    if let Some((path, name)) = &replacing {
        settings::remove_cookies(
            &jar,
            Some(domain.trim_start_matches('.')),
            Some(path),
            Some(name),
        );
    }
    settings::set_cookie(&jar, &domain, &raw)?;
    settings::save_cookies(&id, &jar)
}

/// Deletes one cookie, a domain's cookies (no `name`), or all of them (no `domain`).
#[tauri::command]
pub fn cookies_remove(
    open: tauri::State<'_, Open>,
    domain: Option<String>,
    path: Option<String>,
    name: Option<String>,
) -> Result<()> {
    let (id, jar) = jar(&open)?;
    settings::remove_cookies(&jar, domain.as_deref(), path.as_deref(), name.as_deref());
    settings::save_cookies(&id, &jar)
}
