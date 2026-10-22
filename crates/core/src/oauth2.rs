//! OAuth 2.0 tokens: getting them (every grant Postman has), keeping them in the keychain per workspace, refreshing
//! them, and a list for the token manager. Requests carry only the settings (`http::auth::OAuth2Auth`); the runner
//! asks here for a token before sending.
//!
//! Browser logins (the code and implicit grants) open the provider's page and wait on a loopback redirect URI, such
//! as `http://localhost:8765/callback`, which must be registered with the provider.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use crate::http::auth::{ClientAuthentication, OAuth2Auth, OAuth2Grant};
use crate::http::{self, Network};
use crate::secrets::SecretStore;

/// Tokens are refreshed this long before they expire.
const MARGIN: u64 = 30;
/// How long a browser login may take.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix time in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    pub obtained_at: u64,
}

impl Token {
    pub fn is_valid(&self) -> bool {
        self.expires_at.is_none_or(|at| now() + MARGIN < at)
    }
}

/// A token in the token manager's list, without its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub key: String,
    pub client_id: String,
    pub token_url: String,
    pub grant: OAuth2Grant,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    pub obtained_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub refreshable: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Identifies the token for these settings: the same provider, client, scope and user share one.
pub fn token_key(auth: &OAuth2Auth) -> String {
    let identity = format!(
        "{:?}\n{}\n{}\n{}\n{}\n{}\n{}",
        auth.grant,
        auth.access_token_url,
        auth.auth_url,
        auth.client_id,
        auth.scope,
        auth.audience,
        auth.username
    );
    let hash = ring::digest::digest(&ring::digest::SHA256, identity.as_bytes());
    hash.as_ref()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn secret_name(workspace: &str, key: &str) -> String {
    format!("{workspace}/oauth2/{key}")
}

fn index_file(workspace: &str) -> Option<PathBuf> {
    let id: String = workspace
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    crate::history::data_dir().map(|d| d.join("oauth2").join(format!("{id}.json")))
}

fn read_index(workspace: &str) -> BTreeMap<String, TokenInfo> {
    index_file(workspace)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_index(workspace: &str, index: &BTreeMap<String, TokenInfo>) {
    if let Some(path) = index_file(workspace) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            path,
            serde_json::to_string_pretty(index).unwrap_or_default(),
        );
    }
}

/// The token stored for these settings in `workspace`, valid or not.
pub fn stored(workspace: &str, auth: &OAuth2Auth, secrets: &dyn SecretStore) -> Option<Token> {
    let text = secrets
        .get(&secret_name(workspace, &token_key(auth)))
        .ok()
        .flatten()?;
    serde_json::from_str(&text).ok()
}

fn store(
    workspace: &str,
    auth: &OAuth2Auth,
    token: &Token,
    secrets: &dyn SecretStore,
) -> Result<(), String> {
    let key = token_key(auth);
    secrets.set(
        &secret_name(workspace, &key),
        &serde_json::to_string(token).map_err(|e| e.to_string())?,
    )?;
    let mut index = read_index(workspace);
    index.insert(
        key.clone(),
        TokenInfo {
            key,
            client_id: auth.client_id.clone(),
            token_url: if auth.access_token_url.is_empty() {
                auth.auth_url.clone()
            } else {
                auth.access_token_url.clone()
            },
            grant: auth.grant,
            scope: token.scope.clone().unwrap_or_else(|| auth.scope.clone()),
            obtained_at: token.obtained_at,
            expires_at: token.expires_at,
            refreshable: token.refresh_token.is_some(),
        },
    );
    write_index(workspace, &index);
    Ok(())
}

/// The workspace's stored tokens, for the token manager.
pub fn list(workspace: &str) -> Vec<TokenInfo> {
    read_index(workspace).into_values().collect()
}

/// Deletes a stored token by its key (see [`token_key`]).
pub fn forget(workspace: &str, key: &str, secrets: &dyn SecretStore) -> Result<(), String> {
    secrets.delete(&secret_name(workspace, key))?;
    let mut index = read_index(workspace);
    index.remove(key);
    write_index(workspace, &index);
    Ok(())
}

/// A token for a request: the stored one while valid, else a refreshed one, else, for grants that need no browser,
/// a new one. Browser grants without a usable token fail with a message saying how to log in.
pub async fn token_for(
    auth: &OAuth2Auth,
    workspace: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
) -> Result<Token, String> {
    let saved = workspace.and_then(|w| stored(w, auth, secrets));
    if let Some(token) = &saved
        && token.is_valid()
    {
        return Ok(token.clone());
    }
    if let Some(refresh) = saved.as_ref().and_then(|t| t.refresh_token.clone())
        && !auth.access_token_url.is_empty()
    {
        match token_request(
            auth,
            &[
                ("grant_type", "refresh_token".into()),
                ("refresh_token", refresh.clone()),
            ],
            network,
        )
        .await
        {
            Ok(mut token) => {
                // Providers may not send a new refresh token; the old one still works then.
                token.refresh_token = token.refresh_token.or(Some(refresh));
                if let Some(w) = workspace {
                    store(w, auth, &token, secrets)?;
                }
                return Ok(token);
            }
            Err(e)
                if !matches!(
                    auth.grant,
                    OAuth2Grant::ClientCredentials | OAuth2Grant::Password
                ) =>
            {
                return Err(format!(
                    "OAuth 2.0: the token expired and could not be refreshed ({e}); log in again"
                ));
            }
            Err(_) => {}
        }
    }
    match auth.grant {
        OAuth2Grant::ClientCredentials | OAuth2Grant::Password => {
            let token = authorize(auth, workspace, secrets, network, &|_| {}).await?;
            Ok(token)
        }
        _ => Err(if saved.is_some() {
            "OAuth 2.0: the access token expired; log in again (Auth tab > Get new access token, or `tuma auth login`)"
                .into()
        } else {
            "OAuth 2.0: no access token yet; log in first (Auth tab > Get new access token, or `tuma auth login`)".into()
        }),
    }
}

/// Gets a new token with the settings' grant, and stores it for `workspace`. Browser grants call `open` with the
/// login URL and wait (up to [`LOGIN_TIMEOUT`]) for the provider to redirect back.
pub async fn authorize(
    auth: &OAuth2Auth,
    workspace: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
    open: &(dyn Fn(&str) + Sync),
) -> Result<Token, String> {
    let token = match auth.grant {
        OAuth2Grant::ClientCredentials => {
            let mut params = vec![("grant_type", "client_credentials".to_string())];
            push_common(auth, &mut params);
            token_request(auth, &params, network).await?
        }
        OAuth2Grant::Password => {
            let mut params = vec![
                ("grant_type", "password".to_string()),
                ("username", auth.username.clone()),
                ("password", auth.password.clone()),
            ];
            push_common(auth, &mut params);
            token_request(auth, &params, network).await?
        }
        OAuth2Grant::AuthorizationCode | OAuth2Grant::Implicit => {
            browser_login(auth, network, open).await?
        }
    };
    if let Some(w) = workspace {
        store(w, auth, &token, secrets)?;
    }
    Ok(token)
}

fn push_common(auth: &OAuth2Auth, params: &mut Vec<(&'static str, String)>) {
    if !auth.scope.is_empty() {
        params.push(("scope", auth.scope.clone()));
    }
    if !auth.audience.is_empty() {
        params.push(("audience", auth.audience.clone()));
    }
}

/// POSTs to the token endpoint and reads the token.
async fn token_request(
    auth: &OAuth2Auth,
    params: &[(&str, String)],
    network: Option<&Network>,
) -> Result<Token, String> {
    if auth.access_token_url.is_empty() {
        return Err("OAuth 2.0: the access token URL is empty".into());
    }
    let mut fields: Vec<http::KeyValue> = params
        .iter()
        .map(|(k, v)| http::KeyValue::new(*k, v))
        .collect();
    let mut request_auth = http::Auth::None;
    match auth.client_authentication {
        ClientAuthentication::Header if !auth.client_secret.is_empty() => {
            // RFC 6749 2.3.1: the id and secret are form-encoded before Basic encoding.
            let enc =
                |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
            request_auth = http::Auth::Basic {
                username: enc(&auth.client_id),
                password: enc(&auth.client_secret),
            };
        }
        _ => {
            fields.push(http::KeyValue::new("client_id", &auth.client_id));
            if !auth.client_secret.is_empty() {
                fields.push(http::KeyValue::new("client_secret", &auth.client_secret));
            }
        }
    }
    let request = http::Request {
        method: "POST".into(),
        url: auth.access_token_url.clone(),
        // Some providers (GitHub) answer with a form unless asked for JSON.
        headers: vec![http::KeyValue::new("Accept", "application/json")],
        body: http::Body::FormUrlEncoded { fields },
        auth: request_auth,
        settings: http::Settings {
            timeout_ms: Some(30_000),
            ..Default::default()
        },
    };
    let response = match network {
        Some(n) => http::send_with(&request, n).await,
        None => http::send(&request).await,
    }
    .map_err(|e| format!("OAuth 2.0: could not reach the token endpoint: {e}"))?;
    let text = String::from_utf8_lossy(&response.body).to_string();
    let json: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        Value::Object(
            url::form_urlencoded::parse(text.as_bytes())
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect(),
        )
    });
    parse_token(&json).map_err(|e| {
        if response.status >= 400 {
            format!(
                "OAuth 2.0: the token endpoint answered {}: {e}",
                response.status
            )
        } else {
            format!("OAuth 2.0: {e}")
        }
    })
}

fn parse_token(json: &Value) -> Result<Token, String> {
    if let Some(error) = json["error"].as_str() {
        let description = json["error_description"]
            .as_str()
            .map(|d| format!(": {d}"))
            .unwrap_or_default();
        return Err(format!("{error}{description}"));
    }
    let text = |k: &str| match &json[k] {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let access_token = text("access_token").ok_or("no access_token in the answer")?;
    let obtained_at = now();
    Ok(Token {
        access_token,
        token_type: text("token_type").unwrap_or_default(),
        refresh_token: text("refresh_token"),
        expires_at: text("expires_in")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|s| obtained_at + s),
        scope: text("scope"),
        id_token: text("id_token"),
        obtained_at,
    })
}

fn random_url_safe(bytes: usize) -> String {
    use ring::rand::SecureRandom;
    let mut buf = vec![0u8; bytes];
    let _ = ring::rand::SystemRandom::new().fill(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// The URL the browser opens, and the PKCE verifier and state to check the answer with.
pub fn login_url(auth: &OAuth2Auth) -> Result<(Url, Option<String>, String), String> {
    let mut url = Url::parse(&auth.auth_url)
        .map_err(|e| format!("OAuth 2.0: the auth URL is not valid: {e}"))?;
    let state = random_url_safe(16);
    let verifier =
        (auth.grant == OAuth2Grant::AuthorizationCode && auth.pkce).then(|| random_url_safe(32));
    {
        let mut q = url.query_pairs_mut();
        q.append_pair(
            "response_type",
            if auth.grant == OAuth2Grant::Implicit {
                "token"
            } else {
                "code"
            },
        );
        q.append_pair("client_id", &auth.client_id);
        q.append_pair("redirect_uri", &auth.redirect_uri);
        q.append_pair("state", &state);
        if !auth.scope.is_empty() {
            q.append_pair("scope", &auth.scope);
        }
        if !auth.audience.is_empty() {
            q.append_pair("audience", &auth.audience);
        }
        if let Some(verifier) = &verifier {
            let challenge = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
            q.append_pair(
                "code_challenge",
                &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge),
            );
            q.append_pair("code_challenge_method", "S256");
        }
    }
    Ok((url, verifier, state))
}

async fn browser_login(
    auth: &OAuth2Auth,
    network: Option<&Network>,
    open: &(dyn Fn(&str) + Sync),
) -> Result<Token, String> {
    let redirect = Url::parse(&auth.redirect_uri)
        .map_err(|e| format!("OAuth 2.0: the redirect URI is not valid: {e}"))?;
    let loopback = matches!(
        redirect.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]")
    );
    if redirect.scheme() != "http" || !loopback {
        return Err(format!(
            "OAuth 2.0: Tuma waits for the login on a loopback redirect URI, such as http://localhost:8765/callback, \
             and \"{}\" is not one; register a loopback one with the provider",
            auth.redirect_uri
        ));
    }
    let port = redirect.port_or_known_default().unwrap_or(80);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("OAuth 2.0: could not listen on port {port} for the login: {e}"))?;
    let (url, verifier, state) = login_url(auth)?;
    open(url.as_str());
    let params = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_redirect(&listener, redirect.path()))
        .await
        .map_err(|_| "OAuth 2.0: the login did not finish within 5 minutes".to_string())??;
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if let Some(error) = get("error") {
        let description = get("error_description")
            .map(|d| format!(": {d}"))
            .unwrap_or_default();
        return Err(format!("OAuth 2.0: the login failed: {error}{description}"));
    }
    if get("state").as_deref() != Some(state.as_str()) {
        return Err("OAuth 2.0: the login answer's state doesn't match; try again".into());
    }
    if auth.grant == OAuth2Grant::Implicit {
        let json = Value::Object(
            params
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        );
        return parse_token(&json).map_err(|e| format!("OAuth 2.0: {e}"));
    }
    let code = get("code").ok_or("OAuth 2.0: the login answer has no code")?;
    let mut fields = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", auth.redirect_uri.clone()),
    ];
    if let Some(verifier) = verifier {
        fields.push(("code_verifier", verifier));
    }
    token_request(auth, &fields, network).await
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Tuma</title><body style=\"font:16px system-ui;\
padding:40px\"><h2>Logged in</h2><p>You can close this tab and go back to Tuma.</p>";

/// For the implicit grant: the token is in the URL fragment, which browsers don't send; this page sends it back.
const FRAGMENT_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Tuma</title><body><script>\
location.replace(location.pathname + '?' + location.hash.slice(1) + '&from_fragment=1')</script>";

/// Answers requests on the loopback listener until the redirect arrives, and returns its query parameters.
async fn wait_for_redirect(
    listener: &tokio::net::TcpListener,
    path: &str,
) -> Result<Vec<(String, String)>, String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 16 * 1024];
        let mut read = 0;
        while read < buf.len() && !buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf[read..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => read += n,
            }
        }
        let head = String::from_utf8_lossy(&buf[..read]).to_string();
        let target = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        let url = Url::parse(&format!("http://localhost{target}")).ok();
        let respond = |body: &str| {
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        let Some(url) = url.filter(|u| u.path() == path) else {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        };
        let params: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if params.is_empty() {
            let _ = stream.write_all(respond(FRAGMENT_PAGE).as_bytes()).await;
            continue;
        }
        let _ = stream.write_all(respond(DONE_PAGE).as_bytes()).await;
        return Ok(params);
    }
}

/// Opens a URL in the default browser.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The OAuth 2.0 settings a request at `location` uses, with variables resolved: its own or inherited from its
/// folders. `None` when it doesn't use OAuth 2.0.
pub fn effective_settings(
    workspace: &crate::workspace::Workspace,
    request: &crate::workspace::files::RequestFile,
    location: Option<&str>,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
) -> Result<Option<OAuth2Auth>, String> {
    let prepared = workspace
        .prepare(request, location, environment, &[], secrets)
        .map_err(|e| e.to_string())?;
    Ok(match prepared.request.auth {
        http::Auth::OAuth2(settings) => Some(settings),
        _ => None,
    })
}
