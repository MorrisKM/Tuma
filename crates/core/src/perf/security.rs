//! Passive security checks: they look at responses the test received anyway and send nothing extra. They point out
//! common configuration gaps (transport, security headers, cookie flags, CORS, information that leaks) for the API's
//! owner to review; they are not a penetration test.

use serde::{Deserialize, Serialize};

use crate::http::Response;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// A short name, the same for every request it applies to.
    pub check: String,
    pub request: String,
    pub detail: String,
    pub advice: String,
}

fn local(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':', '?'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "localhost"
        || host.starts_with("127.")
        || host == "[::1]"
        || host.ends_with(".localhost")
        || host.ends_with(".test")
}

/// The passive checks for one response of the request `name`, sent to `url`.
pub fn check(name: &str, url: &str, response: &Response, authenticated: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |severity, check: &str, detail: String, advice: &str| {
        out.push(Finding {
            severity,
            check: check.into(),
            request: name.into(),
            detail,
            advice: advice.into(),
        })
    };
    let header = |name: &str| {
        response
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    };
    let https = url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://");
    let content_type = header("content-type").unwrap_or("").to_ascii_lowercase();
    let html = content_type.contains("text/html");

    if !https && !local(url) {
        add(
            Severity::Medium,
            "Not HTTPS",
            format!("{url} is plain HTTP"),
            "Serve the API over HTTPS only, and redirect HTTP to it.",
        );
    }
    if https && header("strict-transport-security").is_none() {
        add(
            Severity::Low,
            "No HSTS",
            "No Strict-Transport-Security header".into(),
            "Send Strict-Transport-Security: max-age=31536000; includeSubDomains.",
        );
    }
    if header("x-content-type-options").is_none_or(|v| !v.eq_ignore_ascii_case("nosniff")) {
        add(
            Severity::Low,
            "No nosniff",
            "No X-Content-Type-Options: nosniff".into(),
            "Send X-Content-Type-Options: nosniff so browsers don't guess content types.",
        );
    }
    if html {
        if header("content-security-policy").is_none() {
            add(
                Severity::Medium,
                "No CSP",
                "An HTML response without Content-Security-Policy".into(),
                "Send a Content-Security-Policy that limits where scripts can come from.",
            );
        }
        let frames = header("x-frame-options").is_some()
            || header("content-security-policy").is_some_and(|c| c.contains("frame-ancestors"));
        if !frames {
            add(
                Severity::Low,
                "Can be framed",
                "No X-Frame-Options or frame-ancestors".into(),
                "Send X-Frame-Options: DENY or a CSP frame-ancestors directive.",
            );
        }
    }

    // CORS
    let origin = header("access-control-allow-origin");
    let credentials =
        header("access-control-allow-credentials").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    if origin == Some("*") && credentials {
        add(
            Severity::High,
            "CORS: any origin with credentials",
            "Access-Control-Allow-Origin: * with Allow-Credentials: true".into(),
            "Allow specific origins when credentials are allowed; browsers should never get both.",
        );
    } else if origin == Some("*") && authenticated {
        add(
            Severity::Info,
            "CORS: any origin",
            "An authenticated endpoint answers Access-Control-Allow-Origin: *".into(),
            "Check that any website may read this response; list the allowed origins otherwise.",
        );
    }

    // Cookies
    for cookie in response
        .headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("set-cookie"))
    {
        let lower = cookie.value.to_ascii_lowercase();
        let cookie_name = cookie.value.split('=').next().unwrap_or("").trim();
        if https && !lower.contains("secure") {
            add(
                Severity::Medium,
                "Cookie without Secure",
                format!("{cookie_name} can be sent over plain HTTP"),
                "Set the Secure flag on cookies.",
            );
        }
        if !lower.contains("httponly") {
            add(
                Severity::Low,
                "Cookie without HttpOnly",
                format!("{cookie_name} is readable by page scripts"),
                "Set HttpOnly on cookies scripts don't need.",
            );
        }
        if !lower.contains("samesite") {
            add(
                Severity::Low,
                "Cookie without SameSite",
                format!("{cookie_name} has no SameSite"),
                "Set SameSite=Lax (or Strict) to limit cross-site sending.",
            );
        }
    }

    // Information that leaks
    for name in [
        "server",
        "x-powered-by",
        "x-aspnet-version",
        "x-aspnetmvc-version",
    ] {
        if let Some(value) = header(name)
            && value.chars().any(|c| c.is_ascii_digit())
        {
            add(
                Severity::Info,
                "Version disclosed",
                format!("{name}: {value}"),
                "Leave software versions out of response headers.",
            );
        }
    }
    if response.status >= 500 {
        let body = String::from_utf8_lossy(&response.body);
        let traces = [
            "Traceback (most recent call last)",
            "at java.",
            "Exception in thread",
            "    at ",
            "stack trace",
            "System.NullReferenceException",
            ".php on line",
        ];
        if traces.iter().any(|t| body.contains(t)) {
            add(
                Severity::Medium,
                "Stack trace in an error",
                format!("A {} response shows internal details", response.status),
                "Return a generic error message and log the details on the server.",
            );
        }
    }
    let query = url
        .split_once('?')
        .map(|(_, q)| q.to_ascii_lowercase())
        .unwrap_or_default();
    for key in [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_token",
    ] {
        if query.split('&').any(|p| p.split('=').next() == Some(key)) {
            add(
                Severity::Medium,
                "Secret in the URL",
                format!("The query has a {key} parameter"),
                "Send credentials in a header or the body; URLs end up in logs and browser history.",
            );
            break;
        }
    }
    if authenticated && response.status < 300 {
        let cache = header("cache-control").unwrap_or("").to_ascii_lowercase();
        if !cache.contains("no-store") && !cache.contains("private") {
            add(
                Severity::Low,
                "Cacheable private data",
                "An authenticated response without Cache-Control: no-store or private".into(),
                "Send Cache-Control: no-store (or private) on responses with user data.",
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Header;

    fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
        Response {
            status,
            headers: headers
                .iter()
                .map(|(n, v)| Header {
                    name: n.to_string(),
                    value: v.to_string(),
                })
                .collect(),
            body: body.to_string().into_bytes().into(),
            reason: String::new(),
            version: "HTTP/1.1".into(),
            url: String::new(),
            remote_addr: None,
            encoded_size: 0,
            timing: Default::default(),
            redirects: vec![],
            sent: crate::http::SentRequest {
                method: "GET".into(),
                url: String::new(),
                headers: vec![],
                body_size: 0,
            },
        }
    }

    fn names(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.check.as_str()).collect()
    }

    #[test]
    fn finds_common_gaps() {
        let r = response(
            200,
            &[
                ("content-type", "application/json"),
                ("access-control-allow-origin", "*"),
                ("access-control-allow-credentials", "true"),
                ("set-cookie", "sid=1; Path=/"),
                ("server", "nginx/1.25.3"),
            ],
            "{}",
        );
        let f = check("Login", "https://api.example.com/login?token=abc", &r, true);
        let n = names(&f);
        for expected in [
            "No HSTS",
            "No nosniff",
            "CORS: any origin with credentials",
            "Cookie without Secure",
            "Cookie without HttpOnly",
            "Cookie without SameSite",
            "Version disclosed",
            "Secret in the URL",
            "Cacheable private data",
        ] {
            assert!(n.contains(&expected), "missing {expected}: {n:?}");
        }
    }

    #[test]
    fn a_well_configured_response_is_clean() {
        let r = response(
            200,
            &[
                ("content-type", "application/json"),
                ("strict-transport-security", "max-age=31536000"),
                ("x-content-type-options", "nosniff"),
                ("cache-control", "no-store"),
                (
                    "set-cookie",
                    "sid=1; Path=/; Secure; HttpOnly; SameSite=Lax",
                ),
            ],
            "{}",
        );
        assert!(check("Me", "https://api.example.com/me", &r, true).is_empty());
        // Local development servers aren't expected to use HTTPS.
        assert!(!names(&check("Me", "http://localhost:3000/me", &r, false)).contains(&"Not HTTPS"));
        let leaky = response(500, &[], "Traceback (most recent call last):\n  File x");
        assert!(
            names(&check("Boom", "http://localhost/x", &leaky, false))
                .contains(&"Stack trace in an error")
        );
    }
}
