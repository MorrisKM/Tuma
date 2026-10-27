//! cURL commands in and out: paste one from a browser's "Copy as cURL" or API docs, or copy a request as one.

use crate::http::{ApiKeyLocation, Auth as HttpAuth, Body as HttpBody, Request};
use crate::workspace::files::{Auth, Body, KeyValue, Part, RequestFile, Scripts};

/// Reads a cURL command into a request. Unknown options are ignored, and listed in the second value. Pre-request
/// scripts in comments above the command, as Tuma writes them when it copies a request as cURL, become the request's
/// pre-request script.
pub fn parse(command: &str) -> Result<(RequestFile, Vec<String>), String> {
    let (scripts, command) = crate::codegen::scripts_from_comments(command);
    let words = split(command)?;
    let mut words = words.into_iter().peekable();
    match words.next().as_deref() {
        Some("curl") => {}
        _ => return Err("not a curl command: it should start with `curl`".into()),
    }

    let mut ignored = Vec::new();
    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut form: Vec<Part> = Vec::new();
    let mut auth = None;
    let mut get = false;
    let mut json = false;
    let mut insecure = false;

    while let Some(word) = words.next() {
        // --opt=value is the same as --opt value.
        let (flag, inline) = match word.split_once('=') {
            Some((f, v)) if word.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (word.clone(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| words.next())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "-X" | "--request" => method = Some(value()?.to_uppercase()),
            "-H" | "--header" => {
                let header = value()?;
                if let Some((name, v)) = header.split_once(':') {
                    headers.push(KeyValue {
                        name: name.trim().into(),
                        value: v.trim().into(),
                        enabled: true,
                    });
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                data.push(value()?)
            }
            "--data-urlencode" => {
                let v = value()?;
                data.push(match v.split_once('=') {
                    Some((name, content)) => format!("{name}={}", url_encode(content)),
                    None => url_encode(&v),
                });
            }
            "--json" => {
                data.push(value()?);
                json = true;
            }
            "-F" | "--form" | "--form-string" => {
                let v = value()?;
                let (name, content) = v.split_once('=').unwrap_or((&v, ""));
                let file = flag != "--form-string" && content.starts_with('@');
                let content = if file {
                    content[1..].split(';').next().unwrap_or_default()
                } else {
                    content
                };
                form.push(Part {
                    name: name.into(),
                    value: content.into(),
                    file,
                    ..Default::default()
                });
            }
            "-u" | "--user" => {
                let v = value()?;
                let (username, password) = v.split_once(':').unwrap_or((&v, ""));
                auth = Some(Auth::Basic {
                    username: username.into(),
                    password: password.into(),
                });
            }
            "-A" | "--user-agent" => headers.push(KeyValue {
                name: "User-Agent".into(),
                value: value()?,
                enabled: true,
            }),
            "-e" | "--referer" => headers.push(KeyValue {
                name: "Referer".into(),
                value: value()?,
                enabled: true,
            }),
            "-b" | "--cookie" => headers.push(KeyValue {
                name: "Cookie".into(),
                value: value()?,
                enabled: true,
            }),
            "--url" => url = Some(value()?),
            "-G" | "--get" => get = true,
            "-I" | "--head" => method = Some("HEAD".into()),
            "-k" | "--insecure" => insecure = true,
            // Accepted and meaningless here: Tuma decompresses, follows redirects and shows everything anyway.
            "--compressed" | "-L" | "--location" | "-s" | "--silent" | "-S" | "--show-error"
            | "-v" | "--verbose" | "-i" | "--include" | "-f" | "--fail" | "-g" | "--globoff" => {}
            other if other.starts_with('-') => ignored.push(other.to_string()),
            _ if url.is_none() => url = Some(word),
            _ => ignored.push(word),
        }
    }

    let mut url = url.ok_or("the command has no URL")?;
    // A Bearer header becomes Bearer auth, which is easier to edit and to move to a variable.
    if auth.is_none()
        && let Some(i) = headers
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case("authorization"))
        && let Some(token) = headers[i].value.strip_prefix("Bearer ")
    {
        auth = Some(Auth::Bearer {
            token: token.trim().into(),
        });
        headers.remove(i);
    }
    if json {
        for (name, value) in [
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ] {
            if !headers.iter().any(|h| h.name.eq_ignore_ascii_case(name)) {
                headers.push(KeyValue {
                    name: name.into(),
                    value: value.into(),
                    enabled: true,
                });
            }
        }
    }

    let joined = data.join("&");
    let body = if !form.is_empty() {
        Body::Multipart { parts: form }
    } else if data.is_empty() {
        Body::None
    } else if get {
        // -G puts the data in the query string.
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&joined);
        Body::None
    } else {
        let content_type = headers
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case("content-type"))
            .map(|i| headers.remove(i).value);
        match content_type {
            Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => Body::Form {
                fields: form_fields(&joined),
            },
            Some(ct) => Body::text(joined, ct.split(';').next().unwrap_or(&ct).trim()),
            // curl would send a form, but JSON without a content type is almost always meant as JSON.
            None if serde_json::from_str::<serde_json::Value>(&joined).is_ok() => {
                Body::Json { content: joined }
            }
            None => Body::Form {
                fields: form_fields(&joined),
            },
        }
    };
    let method = method.unwrap_or_else(|| {
        if body.is_none() {
            "GET".into()
        } else {
            "POST".into()
        }
    });
    let file = RequestFile {
        method,
        url,
        headers,
        body,
        auth,
        settings: crate::http::Settings {
            verify_tls: !insecure,
            ..Default::default()
        },
        scripts: Scripts {
            pre_request: joined_scripts(&scripts),
            test: String::new(),
        },
        ..RequestFile::default()
    };
    Ok((file, ignored))
}

/// One script from several: one alone as it is; more (a collection's or folder's too) each under its label, as
/// they ran, so ones the request inherits where it is pasted can be told apart and deleted.
fn joined_scripts(scripts: &[crate::script::Script]) -> String {
    match scripts {
        [] => String::new(),
        [one] => one.source.clone(),
        many => many
            .iter()
            .map(|s| format!("// {} pre-request script\n{}", s.label, s.source))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn form_fields(data: &str) -> Vec<KeyValue> {
    url::form_urlencoded::parse(data.as_bytes())
        .map(|(name, value)| KeyValue {
            name: name.into_owned(),
            value: value.into_owned(),
            enabled: true,
        })
        .collect()
}

fn url_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// Splits a command line the way a POSIX shell does: quotes, backslash escapes, `$'...'` and line continuations.
/// Windows `^` continuations from "Copy as cURL (cmd)" are handled too.
fn split(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => {
                    word.push(next);
                    in_word = true;
                }
                None => {}
            },
            '^' if matches!(chars.peek(), Some('\n' | '\r')) => {
                while matches!(chars.peek(), Some('\n' | '\r')) {
                    chars.next();
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("unclosed ' quote".into()),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => word.push('\n'),
                            Some('t') => word.push('\t'),
                            Some('r') => word.push('\r'),
                            Some(other) => word.push(other),
                            None => return Err("unclosed $' quote".into()),
                        },
                        Some(c) => word.push(c),
                        None => return Err("unclosed $' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some('\n') => {}
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err("unclosed \" quote".into()),
                        },
                        Some(c) => word.push(c),
                        None => return Err("unclosed \" quote".into()),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// Quotes a word for a POSIX shell.
fn quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
    {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', r#"'\''"#))
}

/// A header or query parameter: name and value.
type Pair = (String, String);

/// The headers a request sends for its auth, as Tuma's engine would add them, and a query parameter for an API key
/// sent in the URL.
pub(crate) fn auth_headers(request: &Request) -> (Vec<Pair>, Option<Pair>) {
    use base64::Engine;
    match &request.auth {
        HttpAuth::Bearer { token } if !token.is_empty() => (
            vec![("Authorization".into(), format!("Bearer {token}"))],
            None,
        ),
        HttpAuth::Basic { username, password } => {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
            (
                vec![("Authorization".into(), format!("Basic {encoded}"))],
                None,
            )
        }
        HttpAuth::ApiKey {
            key,
            value,
            location: ApiKeyLocation::Header,
        } if !key.is_empty() => (vec![(key.clone(), value.clone())], None),
        HttpAuth::ApiKey {
            key,
            value,
            location: ApiKeyLocation::Query,
        } if !key.is_empty() => (vec![], Some((key.clone(), value.clone()))),
        // A token there is once it has been fetched, as it is for a snippet with auth.
        HttpAuth::OAuth2(oauth) => match &oauth.access_token {
            Some(token) => (
                vec![("Authorization".into(), prefixed(&oauth.prefix, token))],
                None,
            ),
            None => (vec![], None),
        },
        HttpAuth::Jwt(jwt) => match crate::http::auth::jwt_token(jwt) {
            Ok(token) => match &jwt.query_param {
                Some(param) => (vec![], Some((param.clone(), token))),
                None => (
                    vec![("Authorization".into(), prefixed(&jwt.prefix, &token))],
                    None,
                ),
            },
            Err(_) => (vec![], None),
        },
        _ => (vec![], None),
    }
}

/// A token after its prefix, such as "Bearer", if it has one.
fn prefixed(prefix: &str, token: &str) -> String {
    if prefix.is_empty() {
        token.to_string()
    } else {
        format!("{prefix} {token}")
    }
}

/// The URL with an API key in the query string, when the auth puts it there.
pub(crate) fn full_url(request: &Request) -> String {
    let (_, query) = auth_headers(request);
    match query {
        Some((key, value)) => {
            let separator = if request.url.contains('?') { '&' } else { '?' };
            format!(
                "{}{separator}{}={}",
                request.url,
                url_encode(&key),
                url_encode(&value)
            )
        }
        None => request.url.clone(),
    }
}

/// The request as a cURL command, one option per line. Variables must already be resolved.
pub fn to_curl(request: &Request) -> String {
    let request = &request.plain();
    let mut lines = Vec::new();
    let method = request.method.to_uppercase();
    let has_body = !matches!(request.body, HttpBody::None);
    let mut first = format!("curl {}", quote(&full_url(request)));
    if method != "GET" && !(method == "POST" && has_body) {
        first = format!("curl -X {method} {}", quote(&full_url(request)));
    }
    lines.push(first);
    let (auth, _) = auth_headers(request);
    for (name, value) in request
        .headers
        .iter()
        .filter(|h| h.enabled && !h.name.is_empty())
        .map(|h| (&h.name, &h.value))
        .chain(auth.iter().map(|(n, v)| (n, v)))
    {
        lines.push(format!("-H {}", quote(&format!("{name}: {value}"))));
    }
    let has_content_type = request
        .headers
        .iter()
        .any(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"));
    match &request.body {
        HttpBody::None => {}
        HttpBody::Raw {
            content,
            content_type,
        } => {
            if !has_content_type && !content_type.is_empty() {
                lines.push(format!(
                    "-H {}",
                    quote(&format!("Content-Type: {content_type}"))
                ));
            }
            lines.push(format!("--data-raw {}", quote(content)));
        }
        HttpBody::FormUrlEncoded { fields } => {
            for f in fields.iter().filter(|f| f.enabled && !f.name.is_empty()) {
                lines.push(format!(
                    "--data-urlencode {}",
                    quote(&format!("{}={}", f.name, f.value))
                ));
            }
        }
        HttpBody::Multipart { parts } => {
            for p in parts.iter().filter(|p| p.enabled && !p.name.is_empty()) {
                let value = if p.file {
                    format!("{}=@{}", p.name, p.value)
                } else {
                    format!("{}={}", p.name, p.value)
                };
                lines.push(format!(
                    "{} {}",
                    if p.file { "-F" } else { "--form-string" },
                    quote(&value)
                ));
            }
        }
        HttpBody::File { path, content_type } => {
            if let (Some(ct), false) = (content_type, has_content_type) {
                lines.push(format!("-H {}", quote(&format!("Content-Type: {ct}"))));
            }
            lines.push(format!(
                "--data-binary {}",
                quote(&format!("@{}", path.display()))
            ));
        }
        HttpBody::Graphql { .. } => unreachable!("to_curl() makes the body plain"),
    }
    if !request.settings.verify_tls {
        lines.push("--insecure".into());
    }
    if request.settings.follow_redirects {
        lines.push("--location".into());
    }
    lines.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::KeyValue as HttpKeyValue;

    #[test]
    fn scripts_copied_with_a_command_come_back_when_it_is_pasted() {
        use crate::codegen::{Language, with_scripts};
        use crate::script::Script;
        let script = |label: &str, source: &str| Script {
            label: label.into(),
            source: source.into(),
        };
        let command = "curl https://api.example.com/users \\\n  -H 'X-Trace: t1'";

        // The request's own script, blank lines and indentation kept.
        let source = "const id = pm.variables.get('id');\n\nif (id) {\n  pm.request.headers.add({ key: 'X-Id', value: id });\n}";
        let copied = with_scripts(command, &[script("request", source)], Language::Curl);
        assert!(copied.starts_with("# Made after running"), "{copied}");
        let (file, ignored) = parse(&copied).unwrap();
        assert!(ignored.is_empty(), "{ignored:?}");
        assert_eq!(file.url, "https://api.example.com/users");
        assert_eq!(file.headers[0].name, "X-Trace");
        assert_eq!(file.scripts.pre_request, source);

        // With a collection's script too: each under its label.
        let copied = with_scripts(
            command,
            &[
                script("collection Users", "pm.environment.set('token', 'x');"),
                script("request", "pm.variables.set('id', '7');"),
            ],
            Language::Curl,
        );
        let (file, _) = parse(&copied).unwrap();
        assert_eq!(
            file.scripts.pre_request,
            "// Collection Users pre-request script\npm.environment.set('token', 'x');\n\n\
             // Request pre-request script\npm.variables.set('id', '7');"
        );

        // A plain command has no script.
        assert_eq!(parse(command).unwrap().0.scripts.pre_request, "");
    }

    #[test]
    fn browser_copy_as_curl() {
        let command = r#"curl 'https://api.example.com/v1/users?page=2' \
  -H 'accept: application/json' \
  -H 'authorization: Bearer abc.def' \
  -H 'content-type: application/json' \
  --data-raw '{"name":"Ann","note":"it'\''s"}' \
  --compressed"#;
        let (file, ignored) = parse(command).unwrap();
        assert!(ignored.is_empty());
        assert_eq!(file.method, "POST");
        assert_eq!(file.url, "https://api.example.com/v1/users?page=2");
        assert_eq!(
            file.auth,
            Some(Auth::Bearer {
                token: "abc.def".into()
            })
        );
        assert_eq!(file.headers.len(), 1);
        assert_eq!(
            file.body,
            Body::Json {
                content: r#"{"name":"Ann","note":"it's"}"#.into()
            }
        );
    }

    #[test]
    fn forms_users_and_methods() {
        let (file, _) =
            parse("curl -X put -u ann:secret -d a=1 -d 'b=two words' http://x/y").unwrap();
        assert_eq!(file.method, "PUT");
        assert_eq!(
            file.auth,
            Some(Auth::Basic {
                username: "ann".into(),
                password: "secret".into()
            })
        );
        assert_eq!(
            file.body,
            Body::Form {
                fields: form_fields("a=1&b=two words")
            }
        );

        let (file, _) =
            parse(r#"curl -F "photo=@/tmp/a.png;type=image/png" -F name=Rex --url=http://x/pets"#)
                .unwrap();
        let Body::Multipart { parts } = &file.body else {
            panic!()
        };
        assert_eq!(
            (parts[0].value.as_str(), parts[0].file),
            ("/tmp/a.png", true)
        );
        assert_eq!(file.url, "http://x/pets");

        let (file, _) = parse("curl -G -d q=rust http://x/search").unwrap();
        assert_eq!(
            (file.method.as_str(), file.url.as_str()),
            ("GET", "http://x/search?q=rust")
        );

        let (file, _) = parse("curl --json '{\"a\":1}' http://x").unwrap();
        assert!(matches!(file.body, Body::Json { .. }));
        assert!(file.headers.iter().any(|h| h.name == "Accept"));
        assert!(parse("wget http://x").is_err());
        assert!(parse("curl -H").is_err());
    }

    #[test]
    fn round_trip() {
        let request = Request {
            method: "PATCH".into(),
            url: "https://x/users/1".into(),
            headers: vec![HttpKeyValue::new("X-Note", "it's here")],
            body: HttpBody::Raw {
                content: r#"{"a": 1}"#.into(),
                content_type: "application/json".into(),
            },
            auth: HttpAuth::ApiKey {
                key: "key".into(),
                value: "k 1".into(),
                location: ApiKeyLocation::Query,
            },
            ..Default::default()
        };
        let command = to_curl(&request);
        assert_eq!(
            command,
            "curl -X PATCH 'https://x/users/1?key=k+1' \\\n  -H 'X-Note: it'\\''s here' \\\n  -H 'Content-Type: application/json' \\\n  --data-raw '{\"a\": 1}' \\\n  --location"
        );
        let (file, _) = parse(&command).unwrap();
        assert_eq!(file.method, "PATCH");
        assert_eq!(file.url, "https://x/users/1?key=k+1");
        assert_eq!(file.headers[0].value, "it's here");
        assert_eq!(
            file.body,
            Body::Json {
                content: r#"{"a": 1}"#.into()
            }
        );
    }
}
