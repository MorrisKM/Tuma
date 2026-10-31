//! Auth end to end against a local server: Digest's challenge, a signed JWT, and OAuth 2.0 (client credentials,
//! password, refresh, and the browser grants with a scripted "browser") through the runner.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, Once};

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tuma_core::http::auth::{ClientAuthentication, DigestAuth, JwtAuth, OAuth2Auth, OAuth2Grant};
use tuma_core::http::{Auth, Request, send};
use tuma_core::oauth2;
use tuma_core::runner::{self, Job};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::files::{self, RequestFile};

/// Token data and settings files go to a temporary folder, never the real data folder.
fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("tuma-auth-data-{}", std::process::id()));
        // SAFETY: set once, before any test reads it.
        unsafe { std::env::set_var("TUMA_DATA_DIR", dir) };
    });
}

#[derive(Default)]
struct State {
    /// code -> PKCE challenge.
    codes: HashMap<String, String>,
    issued: usize,
    /// For tokens from a login request: how many were given, the one accepted, and whether logins fail.
    logins: usize,
    valid: String,
    login_down: bool,
}

type Shared = Arc<Mutex<State>>;

fn md5_hex(text: &str) -> String {
    use md5::Digest;
    md5::Md5::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn handle(
    state: Shared,
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    let auth = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let reply = |status: u16, body: Value| {
        Ok(hyper::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap())
    };
    match parts.uri.path() {
        "/digest" => {
            let nonce = "abc123";
            let challenge =
                format!(r#"Digest realm="test", qop="auth", nonce="{nonce}", algorithm=MD5"#);
            let Some(params) = auth.strip_prefix("Digest ") else {
                return Ok(hyper::Response::builder()
                    .status(401)
                    .header("www-authenticate", challenge)
                    .body(Full::new(Bytes::new()))
                    .unwrap());
            };
            let get = |k: &str| {
                params
                    .split(", ")
                    .find_map(|p| p.strip_prefix(&format!("{k}=")))
                    .map(|v| v.trim_matches('"').to_string())
                    .unwrap_or_default()
            };
            let ha1 = md5_hex("ann:test:pw");
            let ha2 = md5_hex(&format!("{}:{}", parts.method, get("uri")));
            let expected = md5_hex(&format!(
                "{ha1}:{nonce}:{}:{}:auth:{ha2}",
                get("nc"),
                get("cnonce")
            ));
            if get("response") == expected {
                reply(200, json!({ "user": get("username") }))
            } else {
                reply(401, json!({ "error": "wrong digest" }))
            }
        }
        "/jwt" => {
            let token = auth.strip_prefix("JWT ").unwrap_or_default();
            let (input, sig) = token.rsplit_once('.').unwrap_or_default();
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"shh");
            let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(sig)
                .unwrap_or_default();
            match ring::hmac::verify(&key, input.as_bytes(), &sig) {
                Ok(()) => reply(200, json!({ "ok": true })),
                Err(_) => reply(401, json!({ "error": "bad signature" })),
            }
        }
        "/authorize" => {
            // The provider's login page: approves at once and redirects back.
            let code = format!("code-{}", state.lock().unwrap().codes.len());
            state.lock().unwrap().codes.insert(
                code.clone(),
                query.get("code_challenge").cloned().unwrap_or_default(),
            );
            let redirect = &query["redirect_uri"];
            let location = if query["response_type"] == "token" {
                format!(
                    "{redirect}#access_token=implicit-1&token_type=bearer&state={}",
                    query["state"]
                )
            } else {
                format!("{redirect}?code={code}&state={}", query["state"])
            };
            Ok(hyper::Response::builder()
                .status(302)
                .header("location", location)
                .body(Full::new(Bytes::new()))
                .unwrap())
        }
        "/token" => {
            let client_ok =
                auth == format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode("client:secret")
                ) || (form.get("client_id").map(String::as_str) == Some("client")
                    && form.get("client_secret").map(String::as_str) == Some("secret"));
            if !client_ok {
                return reply(
                    401,
                    json!({ "error": "invalid_client", "error_description": "bad client credentials" }),
                );
            }
            let ok = match form.get("grant_type").map(String::as_str) {
                Some("client_credentials") => true,
                Some("password") => form.get("password").map(String::as_str) == Some("pw"),
                Some("refresh_token") => {
                    form.get("refresh_token").map(String::as_str) == Some("refresh-1")
                }
                Some("authorization_code") => {
                    let challenge = state.lock().unwrap().codes.remove(&form["code"]);
                    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
                    challenge
                        == Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
                }
                _ => false,
            };
            if !ok {
                return reply(400, json!({ "error": "invalid_grant" }));
            }
            let mut s = state.lock().unwrap();
            s.issued += 1;
            // Expires at once, so the next use refreshes it.
            reply(
                200,
                json!({ "access_token": format!("tok-{}", s.issued), "token_type": "Bearer", "expires_in": 0, "refresh_token": "refresh-1" }),
            )
        }
        "/login" => {
            let mut s = state.lock().unwrap();
            if s.login_down {
                reply(401, json!({}))
            } else {
                s.logins += 1;
                let jwt = format!("login-{}", s.logins);
                s.valid = jwt.clone();
                reply(200, json!({ "data": { "session": { "jwt": jwt } } }))
            }
        }
        "/guarded" => {
            let valid = state.lock().unwrap().valid.clone();
            if auth == format!("Bearer {valid}") {
                reply(200, json!({ "token": valid }))
            } else {
                reply(401, json!({ "refused": auth }))
            }
        }
        "/me" => match auth.strip_prefix("Bearer ") {
            Some(token) => reply(200, json!({ "token": token })),
            None => reply(401, json!({})),
        },
        _ => reply(404, json!({})),
    }
}

async fn server() -> (String, Shared) {
    let state = Shared::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shared = state.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let state = shared.clone();
            tokio::spawn(async move {
                let _ = auto::Builder::new(TokioExecutor::new())
                    .serve_connection(
                        TokioIo::new(stream),
                        service_fn(move |r| handle(state.clone(), r)),
                    )
                    .await;
            });
        }
    });
    (format!("http://{addr}"), state)
}

#[tokio::test]
async fn digest_answers_the_challenge() {
    let (base, _) = server().await;
    let request = |password: &str| Request {
        method: "GET".into(),
        url: format!("{base}/digest?x=1"),
        auth: Auth::Digest(DigestAuth {
            username: "ann".into(),
            password: password.into(),
        }),
        ..Default::default()
    };
    let response = send(&request("pw")).await.unwrap();
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    // A wrong password is asked once and then shown as the server's 401, not retried forever.
    assert_eq!(send(&request("nope")).await.unwrap().status, 401);
}

#[tokio::test]
async fn jwt_is_signed_and_sent() {
    let (base, _) = server().await;
    let request = Request {
        method: "GET".into(),
        url: format!("{base}/jwt"),
        auth: Auth::Jwt(JwtAuth {
            secret: "shh".into(),
            payload: r#"{"sub": "ann"}"#.into(),
            prefix: "JWT".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(send(&request).await.unwrap().status, 200);
}

fn oauth(base: &str, grant: OAuth2Grant) -> OAuth2Auth {
    OAuth2Auth {
        grant,
        auth_url: format!("{base}/authorize"),
        access_token_url: format!("{base}/token"),
        client_id: "client".into(),
        client_secret: "secret".into(),
        scope: "read".into(),
        username: "ann".into(),
        password: "pw".into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn oauth2_tokens_through_the_runner() {
    isolate();
    let (base, _) = server().await;
    let secrets = MemoryStore::default();
    let dir = std::env::temp_dir().join(format!("tuma-auth-ws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = tuma_core::workspace::Workspace::create(&dir, "Auth").unwrap();
    let request = RequestFile {
        url: format!("{base}/me"),
        auth: Some(files::Auth::OAuth2(oauth(
            &base,
            OAuth2Grant::ClientCredentials,
        ))),
        ..Default::default()
    };
    let send = || {
        runner::execute(Job {
            workspace: Some(&ws),
            request: &request,
            path: None,
            context: None,
            environment: None,
            overrides: &[],
            extra_checks: None,
            secrets: &secrets,
            history: None,
            source: "test",
            allow_host: None,
            scripting: Default::default(),
            network: None,
            observer: None,
        })
    };
    // Client credentials: fetched without asking.
    let first = send().await;
    let body: Value = serde_json::from_slice(&first.response.as_ref().unwrap().body).unwrap();
    assert_eq!(body["token"], "tok-1");
    // It expired at once, so the next request refreshes it.
    let second = send().await;
    let body: Value = serde_json::from_slice(&second.response.as_ref().unwrap().body).unwrap();
    assert_eq!(body["token"], "tok-2");
    let listed = oauth2::list(ws.id());
    assert_eq!(listed.len(), 1);
    assert!(listed[0].refreshable);
    oauth2::forget(ws.id(), &listed[0].key, &secrets).unwrap();
    assert!(oauth2::list(ws.id()).is_empty());

    // The code grant needs a login first, and the error says so.
    let request = RequestFile {
        auth: Some(files::Auth::OAuth2(oauth(
            &base,
            OAuth2Grant::AuthorizationCode,
        ))),
        ..request.clone()
    };
    let execution = runner::execute(Job {
        workspace: Some(&ws),
        request: &request,
        path: None,
        context: None,
        environment: None,
        overrides: &[],
        extra_checks: None,
        secrets: &secrets,
        history: None,
        source: "test",
        allow_host: None,
        scripting: Default::default(),
        network: None,
        observer: None,
    })
    .await;
    let failure = execution.response.unwrap_err();
    assert_eq!(failure.kind, "auth");
    assert!(
        failure.message.contains("log in first"),
        "{}",
        failure.message
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Plays the browser: opens the login URL without following redirects, then visits where the provider sends it,
/// running the fragment page's script by hand for the implicit grant.
fn browser(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let get = |url: String| Request {
                method: "GET".into(),
                url,
                settings: tuma_core::http::Settings {
                    follow_redirects: false,
                    ..Default::default()
                },
                ..Default::default()
            };
            let login = send(&get(url)).await.unwrap();
            let location = login
                .headers
                .iter()
                .find(|h| h.name == "location")
                .unwrap()
                .value
                .clone();
            match location.split_once('#') {
                Some((path, fragment)) => {
                    let page = send(&get(path.to_string())).await.unwrap();
                    assert!(String::from_utf8_lossy(&page.body).contains("location.hash"));
                    send(&get(format!("{path}?{fragment}&from_fragment=1")))
                        .await
                        .unwrap();
                }
                None => {
                    send(&get(location)).await.unwrap();
                }
            }
        });
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn oauth2_browser_and_password_grants() {
    isolate();
    let (base, _) = server().await;
    let secrets = MemoryStore::default();

    let mut code = oauth(&base, OAuth2Grant::AuthorizationCode);
    code.redirect_uri = "http://localhost:18931/callback".into();
    let token = oauth2::authorize(&code, Some("ws-browser"), &secrets, None, &browser)
        .await
        .unwrap();
    assert!(token.access_token.starts_with("tok-"));
    assert_eq!(token.refresh_token.as_deref(), Some("refresh-1"));
    assert!(oauth2::stored("ws-browser", &code, &secrets).is_some());

    let mut implicit = oauth(&base, OAuth2Grant::Implicit);
    implicit.redirect_uri = "http://localhost:18932/callback".into();
    let token = oauth2::authorize(&implicit, None, &secrets, None, &browser)
        .await
        .unwrap();
    assert_eq!(token.access_token, "implicit-1");

    // Credentials in the body instead of a Basic header; a wrong password is the provider's error, in words.
    let mut password = oauth(&base, OAuth2Grant::Password);
    password.client_authentication = ClientAuthentication::Body;
    assert!(
        oauth2::authorize(&password, None, &secrets, None, &|_| {})
            .await
            .is_ok()
    );
    password.password = "wrong".into();
    let error = oauth2::authorize(&password, None, &secrets, None, &|_| {})
        .await
        .unwrap_err();
    assert!(
        error.contains("400") && error.contains("invalid_grant"),
        "{error}"
    );

    // A redirect URI Tuma can't listen on is refused before the browser opens.
    let mut remote = oauth(&base, OAuth2Grant::AuthorizationCode);
    remote.redirect_uri = "https://oauth.pstmn.io/v1/callback".into();
    let error = oauth2::authorize(&remote, None, &secrets, None, &|_| panic!("opened"))
        .await
        .unwrap_err();
    assert!(error.contains("loopback"), "{error}");
}

#[tokio::test]
async fn a_token_from_another_requests_response() {
    isolate();
    let (base, state) = server().await;
    let secrets = MemoryStore::default();
    let dir = std::env::temp_dir().join(format!("tuma-auth-token-ws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ws = tuma_core::workspace::Workspace::create(&dir, "Auth").unwrap();
    let login = RequestFile {
        url: format!("{base}/login"),
        ..Default::default()
    };
    let collection = ws.create_folder(None, "Api").unwrap();
    let login_path = ws.create_request(&collection, "Login", &login).unwrap();

    let request = |path: &str| RequestFile {
        url: format!("{base}/guarded"),
        auth: Some(files::Auth::RequestToken(
            tuma_core::http::RequestTokenAuth {
                request: login_path.clone(),
                path: path.into(),
            },
        )),
        ..Default::default()
    };
    let send = |request: RequestFile| {
        let ws = &ws;
        let secrets = &secrets;
        async move {
            runner::execute(Job {
                workspace: Some(ws),
                request: &request,
                path: None,
                context: None,
                environment: None,
                overrides: &[],
                extra_checks: None,
                secrets,
                history: None,
                source: "test",
                allow_host: None,
                scripting: Default::default(),
                network: None,
                observer: None,
            })
            .await
        }
    };
    let jwt = "$.data.session.jwt";
    let logins = || state.lock().unwrap().logins;
    let token_of = |response: &tuma_core::http::Response| -> String {
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        body["token"].as_str().unwrap_or_default().to_string()
    };

    // The first send logs in; the next reuses the token.
    let done = send(request(jwt)).await;
    assert_eq!(token_of(done.response.as_ref().unwrap()), "login-1");
    let done = send(request(jwt)).await;
    assert_eq!(token_of(done.response.as_ref().unwrap()), "login-1");
    assert_eq!(logins(), 1);

    // The token expires: a 401 gets a new one, and the request succeeds on the retry.
    state.lock().unwrap().valid = "expired".into();
    let done = send(request(jwt)).await;
    let response = done.response.as_ref().unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(token_of(response), "login-2");
    assert_eq!(logins(), 2);

    // Logging in fails while renewing: it says so.
    state.lock().unwrap().valid = "expired".into();
    state.lock().unwrap().login_down = true;
    let done = send(request(jwt)).await;
    let message = done.response.unwrap_err().message;
    assert!(
        message.contains("refused") && message.contains("401"),
        "{message}"
    );
    state.lock().unwrap().login_down = false;

    // The refused token was dropped, so this logs in up front.
    let before = logins();
    let done = send(request(jwt)).await;
    assert_eq!(done.response.as_ref().unwrap().status, 200);
    assert_eq!(logins(), before + 1);

    // Changing the login request retires the saved token.
    let before = logins();
    let mut changed = login.clone();
    changed.url = format!("{base}/login?user=other");
    ws.write_request(&login_path, &changed).unwrap();
    let done = send(request(jwt)).await;
    assert_eq!(done.response.as_ref().unwrap().status, 200);
    assert_eq!(logins(), before + 1);

    // A missing path stops the send and says why.
    let done = send(request("$.nope")).await;
    assert!(
        done.response
            .unwrap_err()
            .message
            .contains("nothing at $.nope")
    );

    // A token request can't get its own token from a request, so a request can't wait on itself.
    ws.write_request(&login_path, &request("$.other")).unwrap();
    let done = send(request("$.other")).await;
    assert!(
        done.response
            .unwrap_err()
            .message
            .contains("gets its own token")
    );
}
