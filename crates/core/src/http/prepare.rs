//! Turns a [`Request`] into what goes on the wire: a parsed URL, the final headers and the encoded body.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use bytes::Bytes;
use http::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use url::Url;

use super::Error;
use super::auth;
use super::model::{ApiKeyLocation, Auth, Body, Part, Request};

pub(crate) struct Prepared {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Bytes,
}

pub(crate) async fn prepare(request: &Request) -> Result<Prepared, Error> {
    let method = parse_method(&request.method)?;
    let mut url = parse_url(&request.url)?;
    let mut headers = HeaderMap::new();
    for header in request.headers.iter().filter(|h| h.is_active()) {
        let (name, value) = header_pair(&header.name, &header.value)?;
        headers.append(name, value);
    }

    let (mut body, content_type) = encode_body(&request.body).await?;
    if let Some(content_type) = content_type
        && !headers.contains_key(CONTENT_TYPE)
    {
        headers.insert(CONTENT_TYPE, header_value(&content_type)?);
    }

    let version = concat!("Tuma/", env!("CARGO_PKG_VERSION"));
    let defaults = [
        (USER_AGENT, version),
        (ACCEPT, "*/*"),
        (ACCEPT_ENCODING, "gzip, deflate, br"),
    ];
    for (name, value) in defaults {
        headers
            .entry(name)
            .or_insert(HeaderValue::from_static(value));
    }
    // Last, because signatures cover the final URL, headers and body.
    apply_auth(&request.auth, &method, &mut url, &mut headers, &mut body)?;

    Ok(Prepared {
        method,
        url,
        headers,
        body,
    })
}

fn parse_method(method: &str) -> Result<Method, Error> {
    let method = method.trim();
    if method.is_empty() {
        return Ok(Method::GET);
    }
    Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|_| Error::InvalidMethod(method.to_string()))
}

/// Parses a URL the way people type them: surrounding spaces are ignored and `http://` is assumed.
pub(crate) fn parse_url(raw: &str) -> Result<Url, Error> {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url = Url::parse(&with_scheme).map_err(|e| Error::InvalidUrl {
        url: raw.to_string(),
        reason: e.to_string(),
    })?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(Error::UnsupportedScheme(other.to_string())),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(Error::InvalidUrl {
            url: raw.to_string(),
            reason: "no host".to_string(),
        });
    }
    Ok(url)
}

fn header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), Error> {
    let name = HeaderName::from_bytes(name.trim().as_bytes())
        .map_err(|_| Error::InvalidHeader(name.to_string()))?;
    Ok((name, header_value(value)?))
}

fn header_value(value: &str) -> Result<HeaderValue, Error> {
    HeaderValue::from_str(value).map_err(|_| Error::InvalidHeader(value.to_string()))
}

fn apply_auth(
    auth: &Auth,
    method: &Method,
    url: &mut Url,
    headers: &mut HeaderMap,
    body: &mut Bytes,
) -> Result<(), Error> {
    match auth {
        Auth::None => {}
        // Answered after the server's challenge; see the client.
        Auth::Digest(_) => {}
        Auth::Aws(aws) => {
            let list: Vec<(String, String)> = headers
                .iter()
                .map(|(n, v)| {
                    (
                        n.to_string(),
                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                    )
                })
                .collect();
            for (name, value) in auth::aws_sign(aws, method.as_str(), url, &list, body, None) {
                let (name, value) = header_pair(&name, &value)?;
                headers.insert(name, value);
            }
        }
        Auth::Jwt(jwt) => {
            let token = auth::jwt_token(jwt).map_err(Error::Auth)?;
            match &jwt.query_param {
                Some(param) => {
                    url.query_pairs_mut().append_pair(param, &token);
                }
                None => {
                    let value = if jwt.prefix.is_empty() {
                        token
                    } else {
                        format!("{} {token}", jwt.prefix)
                    };
                    headers.insert(AUTHORIZATION, header_value(&value)?);
                }
            }
        }
        Auth::OAuth1(oauth) => {
            let is_form = headers
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("application/x-www-form-urlencoded"));
            let form: Vec<(String, String)> = if is_form {
                url::form_urlencoded::parse(body)
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            } else {
                vec![]
            };
            let params = auth::oauth1_params(oauth, method.as_str(), url, &form, None, None);
            match oauth.location {
                auth::OAuth1Location::Header => {
                    headers.insert(
                        AUTHORIZATION,
                        header_value(&auth::oauth1_header(&oauth.realm, &params))?,
                    );
                }
                auth::OAuth1Location::Body if is_form => {
                    let mut text = String::from_utf8_lossy(body).into_owned();
                    let extra: String = url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(&params)
                        .finish();
                    if !text.is_empty() {
                        text.push('&');
                    }
                    text.push_str(&extra);
                    *body = Bytes::from(text);
                }
                _ => {
                    url.query_pairs_mut().extend_pairs(&params);
                }
            }
        }
        Auth::OAuth2(oauth) => match &oauth.access_token {
            Some(token) => {
                let value = if oauth.prefix.is_empty() {
                    token.clone()
                } else {
                    format!("{} {token}", oauth.prefix)
                };
                headers.insert(AUTHORIZATION, header_value(&value)?);
            }
            None => {
                return Err(Error::Auth(
                    "OAuth 2.0: there is no access token yet; get one from the Auth tab, or with `tuma auth login`".into(),
                ));
            }
        },
        Auth::Bearer { token } if !token.is_empty() => {
            headers.insert(AUTHORIZATION, header_value(&format!("Bearer {token}"))?);
        }
        Auth::Bearer { .. } => {}
        Auth::Basic { username, password } => {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
            headers.insert(AUTHORIZATION, header_value(&format!("Basic {encoded}"))?);
        }
        Auth::ApiKey { key, .. } if key.is_empty() => {}
        Auth::ApiKey {
            key,
            value,
            location: ApiKeyLocation::Header,
        } => {
            let (name, value) = header_pair(key, value)?;
            headers.insert(name, value);
        }
        Auth::ApiKey {
            key,
            value,
            location: ApiKeyLocation::Query,
        } => {
            url.query_pairs_mut().append_pair(key, value);
        }
    }
    Ok(())
}

/// Returns the body bytes and the content type it implies.
async fn encode_body(body: &Body) -> Result<(Bytes, Option<String>), Error> {
    Ok(match body {
        Body::None => (Bytes::new(), None),
        Body::Graphql {
            query,
            variables,
            operation_name,
        } => (
            Bytes::from(
                Body::graphql_json(query, variables, operation_name.as_deref())
                    .map_err(Error::InvalidBody)?,
            ),
            Some("application/json".into()),
        ),
        Body::Raw {
            content,
            content_type,
        } => {
            let content_type = (!content_type.is_empty()).then(|| content_type.clone());
            (Bytes::from(content.clone()), content_type)
        }
        Body::FormUrlEncoded { fields } => {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            for field in fields.iter().filter(|f| f.is_active()) {
                form.append_pair(&field.name, &field.value);
            }
            (
                Bytes::from(form.finish()),
                Some("application/x-www-form-urlencoded".to_string()),
            )
        }
        Body::Multipart { parts } => {
            let boundary = boundary();
            let body = encode_multipart(parts, &boundary).await?;
            (
                body,
                Some(format!("multipart/form-data; boundary={boundary}")),
            )
        }
        Body::File { path, content_type } => {
            let contents = read_file(path).await?;
            let content_type = content_type.clone().unwrap_or_else(|| guess_type(path));
            (contents, Some(content_type))
        }
    })
}

async fn encode_multipart(parts: &[Part], boundary: &str) -> Result<Bytes, Error> {
    let mut out = Vec::new();
    for part in parts.iter().filter(|p| p.enabled && !p.name.is_empty()) {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let name = escape_quoted(&part.name);
        if part.file {
            let path = std::path::Path::new(&part.value);
            let filename = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let content_type = part
                .content_type
                .clone()
                .unwrap_or_else(|| guess_type(path));
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
                    escape_quoted(&filename)
                )
                .as_bytes(),
            );
            out.extend_from_slice(&read_file(path).await?);
        } else {
            out.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
            );
            if let Some(content_type) = &part.content_type {
                out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(part.value.as_bytes());
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(Bytes::from(out))
}

/// Escapes a field or file name the way browsers do (WHATWG multipart/form-data encoding).
fn escape_quoted(name: &str) -> String {
    name.replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn boundary() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("----TumaFormBoundary{nanos:x}{count:x}")
}

fn guess_type(path: &std::path::Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

async fn read_file(path: &std::path::Path) -> Result<Bytes, Error> {
    tokio::fs::read(path)
        .await
        .map(Bytes::from)
        .map_err(|source| Error::File {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::KeyValue;

    #[test]
    fn url_without_scheme_gets_http() {
        assert_eq!(
            parse_url("  localhost:3000/users?a=1 ").unwrap().as_str(),
            "http://localhost:3000/users?a=1"
        );
    }

    #[test]
    fn rejects_other_schemes() {
        assert!(matches!(
            parse_url("ftp://example.com"),
            Err(Error::UnsupportedScheme(_))
        ));
    }

    #[test]
    fn methods_are_upper_cased_and_custom_ones_allowed() {
        assert_eq!(parse_method("post").unwrap(), Method::POST);
        assert_eq!(parse_method("PURGE").unwrap().as_str(), "PURGE");
        assert!(parse_method("BAD METHOD").is_err());
    }

    #[tokio::test]
    async fn basic_auth_and_default_headers() {
        let request = Request {
            url: "example.com".into(),
            auth: Auth::Basic {
                username: "ann".into(),
                password: "secret".into(),
            },
            headers: vec![
                KeyValue::new("Accept", "application/json"),
                KeyValue {
                    enabled: false,
                    ..KeyValue::new("X-Off", "1")
                },
            ],
            ..Default::default()
        };
        let prepared = prepare(&request).await.unwrap();
        assert_eq!(prepared.headers[AUTHORIZATION], "Basic YW5uOnNlY3JldA==");
        assert_eq!(prepared.headers[ACCEPT], "application/json");
        assert!(prepared.headers.contains_key(USER_AGENT));
        assert!(!prepared.headers.contains_key("x-off"));
    }

    #[tokio::test]
    async fn api_key_in_query() {
        let request = Request {
            url: "https://example.com/a?b=1".into(),
            auth: Auth::ApiKey {
                key: "api_key".into(),
                value: "k y".into(),
                location: ApiKeyLocation::Query,
            },
            ..Default::default()
        };
        assert_eq!(
            prepare(&request).await.unwrap().url.as_str(),
            "https://example.com/a?b=1&api_key=k+y"
        );
    }

    #[tokio::test]
    async fn user_content_type_wins_over_the_body_type() {
        let request = Request {
            url: "example.com".into(),
            headers: vec![KeyValue::new("content-type", "application/vnd.api+json")],
            body: Body::Raw {
                content: "{}".into(),
                content_type: "application/json".into(),
            },
            ..Default::default()
        };
        assert_eq!(
            prepare(&request).await.unwrap().headers[CONTENT_TYPE],
            "application/vnd.api+json"
        );
    }

    #[tokio::test]
    async fn multipart_escapes_names() {
        let parts = vec![Part {
            name: "a\"b".into(),
            value: "1".into(),
            ..Default::default()
        }];
        let body = encode_multipart(&parts, "XX").await.unwrap();
        assert_eq!(
            &body[..],
            &b"--XX\r\nContent-Disposition: form-data; name=\"a%22b\"\r\n\r\n1\r\n--XX--\r\n"[..]
        );
    }
}
