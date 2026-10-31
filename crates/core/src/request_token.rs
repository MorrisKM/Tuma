//! Auth whose bearer token comes from another saved request's response, such as a login endpoint.

use crate::http::{self, Network};
use crate::secrets::SecretStore;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Sends the saved request at `path` with its variables and auth, without its scripts, and returns its body.
/// Answers other than 2xx are errors, since they hold no token.
pub async fn fetch(
    workspace: &Workspace,
    path: &str,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
) -> Result<String, String> {
    if path.is_empty() {
        return Err("choose the request that returns the token in the Auth tab".into());
    }
    let file = workspace
        .read_request(path)
        .map_err(|e| format!("token request \"{path}\": {e}"))?;
    let mut prepared = workspace
        .prepare(&file, Some(path), environment, &[], secrets)
        .map_err(|e| format!("token request \"{path}\": {e}"))?;
    if let http::Auth::RequestToken(_) = prepared.request.auth {
        return Err(format!(
            "token request \"{path}\" gets its own token from a request, so it can't be used for one: give it another type of auth"
        ));
    }
    if let http::Auth::OAuth2(oauth) = &mut prepared.request.auth {
        let token = crate::oauth2::token_for(oauth, Some(workspace.id()), secrets, network).await?;
        oauth.access_token = Some(token.access_token);
    }
    let response = match network {
        Some(network) => http::send_with(&prepared.request, network).await,
        None => http::send(&prepared.request).await,
    }
    .map_err(|e| format!("token request \"{path}\": {e}"))?;
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "token request \"{path}\" answered {} {}",
            response.status, response.reason
        ));
    }
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

/// The token at a JSON path in a response body; an empty path means the whole body.
pub fn pick(body: &str, path: &str) -> Result<String, String> {
    if path.trim().is_empty() {
        let token = body.trim();
        return if token.is_empty() {
            Err("the token request's response is empty".into())
        } else {
            Ok(token.to_string())
        };
    }
    let json: Value = serde_json::from_str(body)
        .map_err(|_| format!("the token request's response is not JSON, so it has no {path}"))?;
    match crate::expect::select(&json, path)?.first() {
        Some(Value::String(token)) if !token.is_empty() => Ok(token.clone()),
        Some(Value::Number(token)) => Ok(token.to_string()),
        Some(_) => Err(format!(
            "{path} in the token request's response is not a token"
        )),
        None => Err(format!(
            "the token request's response has nothing at {path}"
        )),
    }
}

/// What [`resolve`] did, for a caller that can retry when the token is refused.
pub struct Resolved {
    pub settings: http::RequestTokenAuth,
    /// The token was saved from an earlier send, so a new one may be accepted where it was not.
    pub reused: bool,
}

/// A saved token, with the login request it came from: changing that request, such as its user, retires the token.
#[derive(Serialize, Deserialize)]
struct Saved {
    source: String,
    token: String,
}

/// One token per environment and per choice of request and path, in the keychain, as OAuth 2.0 tokens are.
fn secret_name(
    workspace: &Workspace,
    settings: &http::RequestTokenAuth,
    environment: Option<&str>,
) -> String {
    format!(
        "{}/request-token/{}/{}/{}",
        workspace.id(),
        environment.unwrap_or(""),
        settings.request,
        settings.path
    )
}

fn source_of(workspace: &Workspace, settings: &http::RequestTokenAuth) -> String {
    let file = workspace
        .read_request(&settings.request)
        .ok()
        .and_then(|file| serde_json::to_string(&file).ok())
        .unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    file.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn saved_token(
    workspace: &Workspace,
    settings: &http::RequestTokenAuth,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
) -> Option<String> {
    let text = secrets
        .get(&secret_name(workspace, settings, environment))
        .ok()
        .flatten()?;
    let saved: Saved = serde_json::from_str(&text).ok()?;
    (saved.source == source_of(workspace, settings)).then_some(saved.token)
}

/// Gets a token by sending the request, and saves it for later sends. Without a keychain it just isn't reused.
pub async fn renew(
    settings: &http::RequestTokenAuth,
    workspace: &Workspace,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
) -> Result<String, String> {
    let name = secret_name(workspace, settings, environment);
    let _ = secrets.delete(&name);
    let body = fetch(workspace, &settings.request, environment, secrets, network).await?;
    let token = pick(&body, &settings.path)?;
    let saved = Saved {
        source: source_of(workspace, settings),
        token: token.clone(),
    };
    if let Ok(text) = serde_json::to_string(&saved) {
        let _ = secrets.set(&name, &text);
    }
    Ok(token)
}

/// Turns token-from-a-request auth into bearer auth: with the saved token, or a new one when there is none. Other
/// auth is left alone.
pub async fn resolve(
    auth: &mut http::Auth,
    workspace: Option<&Workspace>,
    environment: Option<&str>,
    secrets: &dyn SecretStore,
    network: Option<&Network>,
) -> Result<Option<Resolved>, String> {
    let http::Auth::RequestToken(settings) = auth else {
        return Ok(None);
    };
    let settings = settings.clone();
    let workspace = workspace.ok_or("getting a token from a request needs an open workspace")?;
    let saved = saved_token(workspace, &settings, environment, secrets);
    let reused = saved.is_some();
    let token = match saved {
        Some(token) => token,
        None => renew(&settings, workspace, environment, secrets, network).await?,
    };
    *auth = http::Auth::Bearer { token };
    Ok(Some(Resolved { settings, reused }))
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn picks_the_token_from_a_response() {
        let body = r#"{"data":{"token":"abc","ttl":60,"user":{}}}"#;
        assert_eq!(pick(body, "$.data.token").unwrap(), "abc");
        assert_eq!(pick(body, "$.data.ttl").unwrap(), "60");
        assert_eq!(pick(" raw-token\n", "").unwrap(), "raw-token");
        assert!(
            pick(body, "$.data.user")
                .unwrap_err()
                .contains("not a token")
        );
        assert!(pick(body, "$.nope").unwrap_err().contains("nothing at"));
        assert!(pick("plain", "$.token").unwrap_err().contains("not JSON"));
    }
}
