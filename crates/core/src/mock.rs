//! Mock servers: a local HTTP server that answers a collection's (or folder's) requests with their saved examples,
//! so a frontend or another service can be built before, or without, the real API.
//!
//! A request is matched by method and path: `{{name}}`, `:name` and `{name}` segments match anything, and literal
//! segments win over them. Of its examples, the one named by an `x-mock-response-name` header or with the status in
//! `x-mock-response-code` is sent, otherwise the first successful one. The workspace is read again for every request,
//! so edits to examples apply at once. Browsers may call it from any origin (CORS).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::docs::split_url;
use crate::workspace::files::Example;
use crate::workspace::{Node, NodeKind, Workspace};

/// A request the mock can answer.
#[derive(Debug, Clone)]
pub struct Route {
    pub name: String,
    /// The request's path in the workspace.
    pub path: String,
    pub method: String,
    /// `None` for a segment that matches anything.
    segments: Vec<Option<String>>,
    pub examples: Vec<Example>,
}

impl Route {
    /// The path it answers, with `{…}` where any value matches: `/users/{…}`.
    pub fn pattern(&self) -> String {
        let parts: Vec<&str> = self
            .segments
            .iter()
            .map(|s| s.as_deref().unwrap_or("{…}"))
            .collect();
        format!("/{}", parts.join("/"))
    }
}

/// One request the mock received, for its log.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MockEvent {
    /// Unix ms.
    pub at: u64,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// The saved request that answered, and which of its examples.
    pub request: Option<String>,
    pub example: Option<String>,
}

/// What the mock sends back.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub request: Option<String>,
    pub example: Option<String>,
}

/// The HTTP requests in `scope` (a collection's or folder's path, or the whole workspace) as routes.
pub fn routes(workspace: &Workspace, scope: Option<&str>) -> Result<Vec<Route>, String> {
    let tree = workspace.tree().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    fn walk(
        workspace: &Workspace,
        nodes: &[Node],
        scope: Option<&str>,
        out: &mut Vec<Route>,
    ) -> Result<(), String> {
        for node in nodes {
            let inside =
                scope.is_none_or(|s| node.path == s || node.path.starts_with(&format!("{s}/")));
            let above = scope.is_some_and(|s| s.starts_with(&format!("{}/", node.path)));
            if node.kind == NodeKind::Request {
                if !inside {
                    continue;
                }
                let file = workspace
                    .read_request(&node.path)
                    .map_err(|e| e.to_string())?;
                if file.is_websocket() || file.is_grpc() {
                    continue;
                }
                let (_, path, ..) = split_url(&file.url);
                out.push(Route {
                    name: node.name.clone(),
                    path: node.path.clone(),
                    method: file.method.to_ascii_uppercase(),
                    segments: path
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .map(|s| (!(s.starts_with('{') && s.ends_with('}'))).then(|| s.to_string()))
                        .collect(),
                    examples: file.examples,
                });
            } else if inside || above {
                walk(workspace, &node.children, scope, out)?;
            }
        }
        Ok(())
    }
    walk(workspace, &tree, scope, &mut out)?;
    Ok(out)
}

/// Answers one request from `routes`. `header` looks a request header up by its lower-case name.
pub fn answer(
    routes: &[Route],
    method: &str,
    path: &str,
    header: impl Fn(&str) -> Option<String>,
) -> Answer {
    let json = |status: u16, body: serde_json::Value| Answer {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_string_pretty(&body).unwrap_or_default(),
        request: None,
        example: None,
    };
    let segments: Vec<String> = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8_lossy()
                .into_owned()
        })
        .collect();
    let method = method.to_ascii_uppercase();
    // How well a route's path fits: None if it doesn't, otherwise the number of literal segments that matched.
    let fit = |r: &Route| -> Option<usize> {
        if r.segments.len() != segments.len() {
            return None;
        }
        let mut literal = 0;
        for (want, got) in r.segments.iter().zip(&segments) {
            match want {
                Some(w) if w == got => literal += 1,
                Some(_) => return None,
                None => {}
            }
        }
        Some(literal)
    };
    let same_method = |r: &Route| r.method == method || (method == "HEAD" && r.method == "GET");
    let Some(route) = routes
        .iter()
        .filter(|r| same_method(r))
        .filter_map(|r| fit(r).map(|f| (f, r)))
        .max_by_key(|(f, _)| *f)
        .map(|(_, r)| r)
    else {
        let allowed: Vec<&str> = routes
            .iter()
            .filter(|r| fit(r).is_some())
            .map(|r| r.method.as_str())
            .collect();
        return if allowed.is_empty() {
            json(
                404,
                serde_json::json!({ "error": format!("No saved request matches {method} {path}") }),
            )
        } else {
            json(
                405,
                serde_json::json!({ "error": format!("{path} answers {}, not {method}", allowed.join(", ")) }),
            )
        };
    };
    let by_name =
        header("x-mock-response-name").and_then(|n| route.examples.iter().find(|e| e.name == n));
    let by_code = header("x-mock-response-code")
        .and_then(|c| c.trim().parse::<u16>().ok())
        .and_then(|c| route.examples.iter().find(|e| e.status == c));
    let chosen = by_name
        .or(by_code)
        .or_else(|| {
            route
                .examples
                .iter()
                .find(|e| (200..300).contains(&e.status))
        })
        .or_else(|| route.examples.first());
    let Some(example) = chosen else {
        let mut a = json(
            501,
            serde_json::json!({
                "error": format!("\"{}\" has no saved examples to answer with", route.name),
                "hint": "Send the request in Tuma and choose Save as example, or add one in its Examples tab.",
            }),
        );
        a.request = Some(route.name.clone());
        return a;
    };
    let mut headers: Vec<(String, String)> = example
        .headers
        .iter()
        .filter(|h| h.enabled && !h.name.eq_ignore_ascii_case("content-length"))
        .map(|h| (h.name.to_ascii_lowercase(), h.value.clone()))
        .collect();
    if !headers.iter().any(|(n, _)| n == "content-type") && !example.body.is_empty() {
        let guess = if example.body.trim_start().starts_with(['{', '[']) {
            "application/json"
        } else {
            "text/plain"
        };
        headers.push(("content-type".into(), guess.into()));
    }
    Answer {
        status: example.status,
        headers,
        body: if method == "HEAD" {
            String::new()
        } else {
            example.body.clone()
        },
        request: Some(route.name.clone()),
        example: Some(example.name.clone()),
    }
}

pub struct Options {
    /// The workspace's folder; it is read again for every request.
    pub root: PathBuf,
    /// A collection's or folder's path; the whole workspace when `None`.
    pub scope: Option<String>,
    /// 0 picks a free port.
    pub port: u16,
    /// Added before every answer, to see how a client copes with a slow API.
    pub delay_ms: u64,
}

/// A running mock server. Dropping it, or calling [`MockServer::stop`], stops it.
pub struct MockServer {
    pub addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
}

impl MockServer {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

pub type EventSink = Arc<dyn Fn(MockEvent) + Send + Sync>;

/// Starts a mock server on 127.0.0.1. Fails if the port is taken or the scope has nothing to serve.
pub async fn start(options: Options, events: EventSink) -> Result<MockServer, String> {
    let workspace = Workspace::open(&options.root).map_err(|e| e.to_string())?;
    if routes(&workspace, options.scope.as_deref())?.is_empty() {
        return Err("there are no HTTP requests to mock here".into());
    }
    let listener = TcpListener::bind(("127.0.0.1", options.port))
        .await
        .map_err(|e| format!("could not listen on port {}: {e}", options.port))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (stop, mut stopped) = oneshot::channel::<()>();
    let options = Arc::new(options);
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else { continue };
            let options = options.clone();
            let events = events.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req| handle(req, options.clone(), events.clone()));
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Ok(MockServer {
        addr,
        stop: Some(stop),
    })
}

async fn handle(
    req: Request<Incoming>,
    options: Arc<Options>,
    events: EventSink,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().as_str().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let answer = if method == "OPTIONS" {
        // A browser's CORS preflight.
        Answer {
            status: 204,
            headers: vec![],
            body: String::new(),
            request: None,
            example: None,
        }
    } else {
        match Workspace::open(&options.root)
            .map_err(|e| e.to_string())
            .and_then(|w| routes(&w, options.scope.as_deref()))
        {
            Ok(routes) => answer(&routes, &method, &path, |name| {
                req.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
            }),
            Err(e) => Answer {
                status: 500,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: format!("The mock could not read the workspace: {e}"),
                request: None,
                example: None,
            },
        }
    };
    if options.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(options.delay_ms)).await;
    }
    events(MockEvent {
        at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default(),
        method,
        path,
        status: answer.status,
        request: answer.request.clone(),
        example: answer.example.clone(),
    });
    let mut response = Response::builder()
        .status(answer.status)
        .header("access-control-allow-origin", "*")
        .header(
            "access-control-allow-methods",
            "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS",
        )
        .header("access-control-allow-headers", "*")
        .header("access-control-expose-headers", "*")
        .header("x-mock", "tuma");
    for (name, value) in &answer.headers {
        response = response.header(name, value);
    }
    Ok(response
        .body(Full::new(Bytes::from(answer.body)))
        .unwrap_or_else(|_| {
            Response::new(Full::new(Bytes::from_static(
                b"The example's headers are not valid",
            )))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::files::KeyValue;

    fn route(method: &str, path: &str, examples: Vec<Example>) -> Route {
        let (_, path, ..) = split_url(path);
        Route {
            name: format!("{method} {path}"),
            path: String::new(),
            method: method.into(),
            segments: path
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| (!(s.starts_with('{') && s.ends_with('}'))).then(|| s.to_string()))
                .collect(),
            examples,
        }
    }

    fn example(name: &str, status: u16, body: &str) -> Example {
        Example {
            name: name.into(),
            status,
            headers: vec![KeyValue {
                name: "Content-Type".into(),
                value: "application/json".into(),
                enabled: true,
            }],
            body: body.into(),
        }
    }

    #[test]
    fn matches_paths_and_picks_examples() {
        let routes = vec![
            route(
                "GET",
                "{{baseUrl}}/users/:id",
                vec![
                    example("Missing", 404, "{}"),
                    example("Found", 200, r#"{"id":1}"#),
                ],
            ),
            route(
                "GET",
                "{{baseUrl}}/users/me",
                vec![example("Me", 200, r#"{"me":true}"#)],
            ),
            route("POST", "{{baseUrl}}/users", vec![]),
        ];
        let none = |_: &str| None;
        // The first successful example, by default.
        let a = answer(&routes, "GET", "/users/42?x=1", none);
        assert_eq!((a.status, a.example.as_deref()), (200, Some("Found")));
        // A literal segment wins over a parameter.
        assert_eq!(
            answer(&routes, "GET", "/users/me", none).example.as_deref(),
            Some("Me")
        );
        // Chosen by code or name.
        let by_code = answer(&routes, "GET", "/users/7", |h| {
            (h == "x-mock-response-code").then(|| "404".into())
        });
        assert_eq!(by_code.status, 404);
        let by_name = answer(&routes, "GET", "/users/7", |h| {
            (h == "x-mock-response-name").then(|| "Missing".into())
        });
        assert_eq!(by_name.status, 404);
        // No examples, the wrong method, and nothing at all.
        assert_eq!(answer(&routes, "POST", "/users", none).status, 501);
        assert_eq!(answer(&routes, "DELETE", "/users", none).status, 405);
        assert_eq!(answer(&routes, "GET", "/nothing", none).status, 404);
        // HEAD is answered like GET, without the body.
        let head = answer(&routes, "HEAD", "/users/me", none);
        assert_eq!((head.status, head.body.as_str()), (200, ""));
    }
}
