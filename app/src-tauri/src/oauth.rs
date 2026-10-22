//! OAuth 2.0 from the app: the token status of the auth being edited, getting a new token (browser grants open the
//! system browser), and the token manager.

use serde::Serialize;
use tuma_core::oauth2::{self, TokenInfo};
use tuma_core::runner;
use tuma_core::workspace::files::RequestFile;

use crate::workspace::{Open, secrets};

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatus {
    key: String,
    /// Unix time in seconds.
    obtained_at: Option<u64>,
    expires_at: Option<u64>,
    refreshable: bool,
    scope: Option<String>,
    /// Whether a token is stored at all.
    present: bool,
}

/// The settings a request (or a folder, with `path` inside it) uses, with variables resolved.
fn settings(
    open: &Open,
    request: &RequestFile,
    path: Option<&str>,
    environment: Option<&str>,
) -> Result<(
    tuma_core::workspace::Workspace,
    tuma_core::http::auth::OAuth2Auth,
)> {
    let ws = open
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("Open a workspace first: tokens are kept per workspace")?;
    let file = RequestFile {
        url: runner::absolute_url(&request.url),
        ..request.clone()
    };
    let location = path
        .map(String::from)
        .or_else(|| runner::implicit_location(&ws, &file.url));
    let settings =
        oauth2::effective_settings(&ws, &file, location.as_deref(), environment, secrets())?
            .ok_or("This request doesn't use OAuth 2.0")?;
    Ok((ws, settings))
}

fn status(
    ws: &tuma_core::workspace::Workspace,
    settings: &tuma_core::http::auth::OAuth2Auth,
) -> TokenStatus {
    let token = oauth2::stored(ws.id(), settings, secrets());
    TokenStatus {
        key: oauth2::token_key(settings),
        obtained_at: token.as_ref().map(|t| t.obtained_at),
        expires_at: token.as_ref().and_then(|t| t.expires_at),
        refreshable: token.as_ref().is_some_and(|t| t.refresh_token.is_some()),
        scope: token.as_ref().and_then(|t| t.scope.clone()),
        present: token.is_some(),
    }
}

#[tauri::command]
pub fn oauth2_status(
    open: tauri::State<'_, Open>,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
) -> Result<TokenStatus> {
    let (ws, settings) = settings(&open, &request, path.as_deref(), environment.as_deref())?;
    Ok(status(&ws, &settings))
}

/// Gets a new token with the request's settings. Browser grants open the system browser and wait for the login.
#[tauri::command]
pub async fn oauth2_authorize(
    open: tauri::State<'_, Open>,
    request: RequestFile,
    path: Option<String>,
    environment: Option<String>,
) -> Result<TokenStatus> {
    let (ws, settings) = settings(&open, &request, path.as_deref(), environment.as_deref())?;
    let network = tuma_core::settings::network_for(Some(&ws), secrets())?;
    let open_browser = |url: &str| {
        if let Err(e) = oauth2::open_in_browser(url) {
            eprintln!("could not open the browser: {e}");
        }
    };
    oauth2::authorize(
        &settings,
        Some(ws.id()),
        secrets(),
        Some(&network),
        &open_browser,
    )
    .await?;
    Ok(status(&ws, &settings))
}

#[tauri::command]
pub fn oauth2_tokens(open: tauri::State<'_, Open>) -> Result<Vec<TokenInfo>> {
    let guard = open.0.lock().unwrap();
    let ws = guard.as_ref().ok_or("No workspace is open")?;
    Ok(oauth2::list(ws.id()))
}

#[tauri::command]
pub fn oauth2_forget(open: tauri::State<'_, Open>, key: String) -> Result<()> {
    let guard = open.0.lock().unwrap();
    let ws = guard.as_ref().ok_or("No workspace is open")?;
    oauth2::forget(ws.id(), &key, secrets())
}
