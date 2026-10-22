//! Auth that signs or answers: Digest, AWS Signature v4, JWT bearer and OAuth 1.0. Their settings are shared with the
//! request files (`workspace::files::Auth`), and `{{variables}}` are resolved before anything here runs.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use ring::{digest, hmac, rand, signature};
use rustls::pki_types::{PrivateKeyDer, pem::PemObject};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DigestAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct AwsAuth {
    pub access_key: String,
    pub secret_key: String,
    /// For temporary credentials.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_token: String,
    /// Such as `us-east-1`.
    pub region: String,
    /// Such as `s3`, `execute-api` or `iam`.
    pub service: String,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum JwtAlgorithm {
    #[default]
    HS256,
    HS384,
    HS512,
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    ES256,
    ES384,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct JwtAuth {
    pub algorithm: JwtAlgorithm,
    /// For HS algorithms, the shared secret; otherwise a PEM private key (PKCS#8, or PKCS#1 for RSA).
    pub secret: String,
    /// The HS secret is base64.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub secret_base64: bool,
    /// The claims, as JSON.
    pub payload: String,
    /// Extra header fields, as JSON, such as `{"kid": "key-1"}`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub headers: String,
    /// Sent as `Authorization: <prefix> <token>`.
    pub prefix: String,
    /// Send the token as this query parameter instead of a header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_param: Option<String>,
}

impl Default for JwtAuth {
    fn default() -> Self {
        Self {
            algorithm: JwtAlgorithm::HS256,
            secret: String::new(),
            secret_base64: false,
            payload: "{}".into(),
            headers: String::new(),
            prefix: "Bearer".into(),
            query_param: None,
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum OAuth1Method {
    #[default]
    #[serde(rename = "HMAC-SHA1")]
    HmacSha1,
    #[serde(rename = "HMAC-SHA256")]
    HmacSha256,
    #[serde(rename = "HMAC-SHA512")]
    HmacSha512,
    #[serde(rename = "PLAINTEXT")]
    Plaintext,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum OAuth1Location {
    #[default]
    Header,
    Query,
    /// In a form body (only for `application/x-www-form-urlencoded` requests).
    Body,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct OAuth1Auth {
    pub consumer_key: String,
    pub consumer_secret: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token_secret: String,
    pub signature_method: OAuth1Method,
    pub location: OAuth1Location,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub realm: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub callback: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub verifier: String,
}

/// RFC 3986 percent-encoding, as OAuth 1.0 and AWS need: everything but letters, digits and `-._~`.
pub(crate) fn encode(text: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(text, SET).to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn hmac_sign(algorithm: hmac::Algorithm, key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(algorithm, key), data)
        .as_ref()
        .to_vec()
}

fn random_hex(bytes: usize) -> String {
    use rand::SecureRandom;
    let mut buf = vec![0u8; bytes];
    let _ = rand::SystemRandom::new().fill(&mut buf);
    hex(&buf)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

// Digest (RFC 7616 and RFC 2617)

/// The `Authorization` header that answers a `WWW-Authenticate: Digest ...` challenge, or `None` if the challenge is
/// not Digest or uses an algorithm Tuma doesn't know.
pub fn digest_authorization(
    challenge: &str,
    auth: &DigestAuth,
    method: &str,
    uri: &str,
    cnonce: Option<&str>,
) -> Option<String> {
    let params = challenge
        .trim()
        .strip_prefix("Digest")
        .or_else(|| challenge.trim().strip_prefix("digest"))?;
    let params = parse_params(params);
    let get = |k: &str| {
        params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    };
    let realm = get("realm").unwrap_or_default();
    let nonce = get("nonce")?;
    let algorithm = get("algorithm").unwrap_or("MD5");
    let (hash, sess): (fn(&str) -> String, bool) = match algorithm.to_ascii_uppercase().as_str() {
        "MD5" => (md5_hex, false),
        "MD5-SESS" => (md5_hex, true),
        "SHA-256" => (|s: &str| sha256_hex(s.as_bytes()), false),
        "SHA-256-SESS" => (|s: &str| sha256_hex(s.as_bytes()), true),
        _ => return None,
    };
    let qop = get("qop").map(|q| {
        if q.split(',').any(|o| o.trim() == "auth") {
            "auth"
        } else {
            ""
        }
    });
    let cnonce = cnonce.map(String::from).unwrap_or_else(|| random_hex(8));
    let nc = "00000001";
    let mut ha1 = hash(&format!("{}:{realm}:{}", auth.username, auth.password));
    if sess {
        ha1 = hash(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = hash(&format!("{method}:{uri}"));
    let response = match qop {
        Some("auth") => hash(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}")),
        _ => hash(&format!("{ha1}:{nonce}:{ha2}")),
    };
    let mut header = format!(
        "Digest username=\"{}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", algorithm={algorithm}, response=\"{response}\"",
        auth.username
    );
    if qop == Some("auth") {
        header.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = get("opaque") {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    Some(header)
}

fn md5_hex(text: &str) -> String {
    use md5::Digest;
    hex(&md5::Md5::digest(text.as_bytes()))
}

/// `a="b", c=d` into pairs, honouring quotes.
fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let name: String =
            std::iter::from_fn(|| chars.next_if(|c| *c != '=' && *c != ',')).collect();
        if name.trim().is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            if chars.next_if_eq(&'"').is_some() {
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => value.extend(chars.next()),
                        '"' => break,
                        c => value.push(c),
                    }
                }
            } else {
                value = std::iter::from_fn(|| chars.next_if(|c| *c != ','))
                    .collect::<String>()
                    .trim()
                    .to_string();
            }
        }
        out.push((name.trim().to_string(), value));
    }
    out
}

// AWS Signature Version 4

/// Signs a request: returns the headers to add (`Authorization`, `X-Amz-Date`, `X-Amz-Content-Sha256` and, with
/// temporary credentials, `X-Amz-Security-Token`). `headers` are the request's other headers; those that matter for
/// signing (`host`, `content-type`, `x-amz-*`) are signed. `time` is the Unix time, for tests.
pub fn aws_sign(
    auth: &AwsAuth,
    method: &str,
    url: &Url,
    headers: &[(String, String)],
    body: &[u8],
    time: Option<u64>,
) -> Vec<(String, String)> {
    let time = time.unwrap_or_else(now);
    let datetime = format_amz_date(time);
    let date = &datetime[..8];
    let payload_hash = sha256_hex(body);

    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    };
    let mut signed: Vec<(String, String)> = vec![
        ("host".into(), host),
        ("x-amz-date".into(), datetime.clone()),
    ];
    if auth.service == "s3" {
        signed.push(("x-amz-content-sha256".into(), payload_hash.clone()));
    }
    if !auth.session_token.is_empty() {
        signed.push(("x-amz-security-token".into(), auth.session_token.clone()));
    }
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if (lower == "content-type" || lower.starts_with("x-amz-"))
            && !signed.iter().any(|(n, _)| *n == lower)
        {
            signed.push((lower, value.trim().to_string()));
        }
    }
    signed.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = signed
        .iter()
        .map(|(n, v)| {
            format!(
                "{n}:{}\n",
                v.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    let signed_names = signed
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";");

    // S3 paths are not encoded twice; every other service encodes each segment again.
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let canonical_path = if auth.service == "s3" {
        path.to_string()
    } else {
        path.split('/')
            .map(|seg| encode(&percent_encoding::percent_decode_str(seg).decode_utf8_lossy()))
            .map(|seg| encode(&seg))
            .collect::<Vec<_>>()
            .join("/")
    };
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (encode(&k), encode(&v)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_request = format!(
        "{}\n{canonical_path}\n{canonical_query}\n{canonical_headers}\n{signed_names}\n{payload_hash}",
        method.to_uppercase()
    );
    let scope = format!("{date}/{}/{}/aws4_request", auth.region, auth.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sign(
        hmac::HMAC_SHA256,
        format!("AWS4{}", auth.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac_sign(hmac::HMAC_SHA256, &k_date, auth.region.as_bytes());
    let k_service = hmac_sign(hmac::HMAC_SHA256, &k_region, auth.service.as_bytes());
    let k_signing = hmac_sign(hmac::HMAC_SHA256, &k_service, b"aws4_request");
    let signature = hex(&hmac_sign(
        hmac::HMAC_SHA256,
        &k_signing,
        string_to_sign.as_bytes(),
    ));

    let mut out = vec![
        (
            "Authorization".to_string(),
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_names}, Signature={signature}",
                auth.access_key
            ),
        ),
        ("X-Amz-Date".to_string(), datetime),
    ];
    if auth.service == "s3" {
        out.push(("X-Amz-Content-Sha256".into(), payload_hash));
    }
    if !auth.session_token.is_empty() {
        out.push(("X-Amz-Security-Token".into(), auth.session_token.clone()));
    }
    out
}

/// `20150830T123600Z`.
fn format_amz_date(time: u64) -> String {
    let days = (time / 86_400) as i64;
    let secs = time % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

// JWT bearer

/// A signed JWT from the settings.
pub fn jwt_token(auth: &JwtAuth) -> Result<String, String> {
    let alg = format!("{:?}", auth.algorithm);
    let mut header = serde_json::json!({ "alg": alg, "typ": "JWT" });
    if !auth.headers.trim().is_empty() {
        let extra: serde_json::Value = serde_json::from_str(&auth.headers)
            .map_err(|e| format!("JWT headers are not valid JSON: {e}"))?;
        let extra = extra
            .as_object()
            .ok_or("JWT headers must be a JSON object")?;
        for (k, v) in extra {
            header[k] = v.clone();
        }
    }
    let payload: serde_json::Value = serde_json::from_str(if auth.payload.trim().is_empty() {
        "{}"
    } else {
        &auth.payload
    })
    .map_err(|e| format!("the JWT payload is not valid JSON: {e}"))?;
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let input = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(payload.to_string().as_bytes())
    );
    let signature = jwt_sign(auth, input.as_bytes())?;
    Ok(format!("{input}.{}", b64(&signature)))
}

fn jwt_sign(auth: &JwtAuth, data: &[u8]) -> Result<Vec<u8>, String> {
    use JwtAlgorithm::*;
    let rng = rand::SystemRandom::new();
    let hmac_alg = match auth.algorithm {
        HS256 => Some(hmac::HMAC_SHA256),
        HS384 => Some(hmac::HMAC_SHA384),
        HS512 => Some(hmac::HMAC_SHA512),
        _ => None,
    };
    if let Some(alg) = hmac_alg {
        let key = if auth.secret_base64 {
            base64::engine::general_purpose::STANDARD
                .decode(auth.secret.trim())
                .map_err(|e| format!("the JWT secret is not valid base64: {e}"))?
        } else {
            auth.secret.as_bytes().to_vec()
        };
        return Ok(hmac_sign(alg, &key, data));
    }
    let key = PrivateKeyDer::from_pem_slice(auth.secret.as_bytes()).map_err(|_| {
        format!(
            "{:?} needs a PEM private key (-----BEGIN PRIVATE KEY-----)",
            auth.algorithm
        )
    })?;
    match auth.algorithm {
        RS256 | RS384 | RS512 | PS256 | PS384 | PS512 => {
            let pair = match &key {
                PrivateKeyDer::Pkcs8(k) => signature::RsaKeyPair::from_pkcs8(k.secret_pkcs8_der()),
                PrivateKeyDer::Pkcs1(k) => signature::RsaKeyPair::from_der(k.secret_pkcs1_der()),
                _ => return Err("an RSA algorithm needs an RSA private key".into()),
            }
            .map_err(|e| format!("the RSA private key can't be used: {e}"))?;
            let padding: &dyn signature::RsaEncoding = match auth.algorithm {
                RS256 => &signature::RSA_PKCS1_SHA256,
                RS384 => &signature::RSA_PKCS1_SHA384,
                RS512 => &signature::RSA_PKCS1_SHA512,
                PS256 => &signature::RSA_PSS_SHA256,
                PS384 => &signature::RSA_PSS_SHA384,
                _ => &signature::RSA_PSS_SHA512,
            };
            let mut out = vec![0u8; pair.public().modulus_len()];
            pair.sign(padding, &rng, data, &mut out)
                .map_err(|e| e.to_string())?;
            Ok(out)
        }
        ES256 | ES384 => {
            let alg = if auth.algorithm == ES256 {
                &signature::ECDSA_P256_SHA256_FIXED_SIGNING
            } else {
                &signature::ECDSA_P384_SHA384_FIXED_SIGNING
            };
            let PrivateKeyDer::Pkcs8(k) = &key else {
                return Err(
                    "an EC key must be PKCS#8 (-----BEGIN PRIVATE KEY-----); convert it with `openssl pkcs8 -topk8 -nocrypt`".into(),
                );
            };
            let pair = signature::EcdsaKeyPair::from_pkcs8(alg, k.secret_pkcs8_der(), &rng)
                .map_err(|e| format!("the EC private key can't be used: {e}"))?;
            Ok(pair
                .sign(&rng, data)
                .map_err(|e| e.to_string())?
                .as_ref()
                .to_vec())
        }
        _ => unreachable!(),
    }
}

// OAuth 1.0 (RFC 5849)

/// The `oauth_*` parameters, signed, for a request. `form` holds the fields of a form body, which are signed too.
/// `timestamp` and `nonce` are for tests.
pub fn oauth1_params(
    auth: &OAuth1Auth,
    method: &str,
    url: &Url,
    form: &[(String, String)],
    timestamp: Option<u64>,
    nonce: Option<&str>,
) -> Vec<(String, String)> {
    let mut oauth: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), auth.consumer_key.clone()),
        (
            "oauth_nonce".into(),
            nonce.map(String::from).unwrap_or_else(|| random_hex(16)),
        ),
        (
            "oauth_signature_method".into(),
            match auth.signature_method {
                OAuth1Method::HmacSha1 => "HMAC-SHA1",
                OAuth1Method::HmacSha256 => "HMAC-SHA256",
                OAuth1Method::HmacSha512 => "HMAC-SHA512",
                OAuth1Method::Plaintext => "PLAINTEXT",
            }
            .into(),
        ),
        (
            "oauth_timestamp".into(),
            timestamp.unwrap_or_else(now).to_string(),
        ),
        ("oauth_version".into(), "1.0".into()),
    ];
    if !auth.token.is_empty() {
        oauth.push(("oauth_token".into(), auth.token.clone()));
    }
    if !auth.callback.is_empty() {
        oauth.push(("oauth_callback".into(), auth.callback.clone()));
    }
    if !auth.verifier.is_empty() {
        oauth.push(("oauth_verifier".into(), auth.verifier.clone()));
    }
    let key = format!(
        "{}&{}",
        encode(&auth.consumer_secret),
        encode(&auth.token_secret)
    );
    let signature = match auth.signature_method {
        OAuth1Method::Plaintext => key,
        method_kind => {
            let mut params: Vec<(String, String)> = oauth
                .iter()
                .cloned()
                .chain(
                    url.query_pairs()
                        .map(|(k, v)| (k.into_owned(), v.into_owned())),
                )
                .chain(form.iter().cloned())
                .map(|(k, v)| (encode(&k), encode(&v)))
                .collect();
            params.sort();
            let normalized = params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            let mut base_url = url.clone();
            base_url.set_query(None);
            base_url.set_fragment(None);
            let base = format!(
                "{}&{}&{}",
                method.to_uppercase(),
                encode(base_url.as_str()),
                encode(&normalized)
            );
            let alg = match method_kind {
                OAuth1Method::HmacSha256 => hmac::HMAC_SHA256,
                OAuth1Method::HmacSha512 => hmac::HMAC_SHA512,
                _ => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            };
            base64::engine::general_purpose::STANDARD.encode(hmac_sign(
                alg,
                key.as_bytes(),
                base.as_bytes(),
            ))
        }
    };
    oauth.push(("oauth_signature".into(), signature));
    oauth
}

/// `OAuth realm="...", oauth_consumer_key="...", ...`.
pub fn oauth1_header(realm: &str, params: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    if !realm.is_empty() {
        parts.push(format!("realm=\"{}\"", encode(realm)));
    }
    parts.extend(
        params
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", encode(k), encode(v))),
    );
    format!("OAuth {}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_rfc2617_example() {
        let challenge = r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let auth = DigestAuth {
            username: "Mufasa".into(),
            password: "Circle Of Life".into(),
        };
        let header =
            digest_authorization(challenge, &auth, "GET", "/dir/index.html", Some("0a4f113b"))
                .unwrap();
        assert!(
            header.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{header}"
        );
        assert!(header.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
        assert!(header.contains("qop=auth, nc=00000001"));
        assert_eq!(
            digest_authorization("Basic realm=x", &auth, "GET", "/", None),
            None
        );
    }

    #[test]
    fn aws_sigv4_example() {
        // https://docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html (the IAM ListUsers example).
        let auth = AwsAuth {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            service: "iam".into(),
            ..Default::default()
        };
        let url =
            Url::parse("https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08").unwrap();
        let headers = vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded; charset=utf-8".into(),
        )];
        let time = 1_440_938_160; // 2015-08-30T12:36:00Z
        let out = aws_sign(&auth, "GET", &url, &headers, b"", Some(time));
        assert_eq!(out[1], ("X-Amz-Date".into(), "20150830T123600Z".into()));
        assert_eq!(
            out[0].1,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, \
             Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
        assert_eq!(format_amz_date(951_782_400), "20000229T000000Z");
    }

    #[test]
    fn jwt_hs256_matches_jwt_io() {
        let auth = JwtAuth {
            secret: "your-256-bit-secret".into(),
            payload: r#"{"sub":"1234567890","name":"John Doe","iat":1516239022}"#.into(),
            ..Default::default()
        };
        assert_eq!(
            jwt_token(&auth).unwrap(),
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
        );
        assert!(
            jwt_token(&JwtAuth {
                payload: "{".into(),
                ..auth.clone()
            })
            .unwrap_err()
            .contains("not valid JSON")
        );
        assert!(
            jwt_token(&JwtAuth {
                algorithm: JwtAlgorithm::RS256,
                ..auth
            })
            .unwrap_err()
            .contains("PEM private key")
        );
    }

    #[test]
    fn jwt_es256_verifies() {
        use signature::KeyPair;
        let rng = rand::SystemRandom::new();
        let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
        let key = signature::EcdsaKeyPair::generate_pkcs8(alg, &rng)
            .unwrap()
            .as_ref()
            .to_vec();
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            base64::engine::general_purpose::STANDARD.encode(&key)
        );
        let auth = JwtAuth {
            algorithm: JwtAlgorithm::ES256,
            secret: pem,
            payload: r#"{"sub":"a"}"#.into(),
            ..Default::default()
        };
        let token = jwt_token(&auth).unwrap();
        let (input, sig) = token.rsplit_once('.').unwrap();
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig)
            .unwrap();
        let pair = signature::EcdsaKeyPair::from_pkcs8(alg, &key, &rng).unwrap();
        signature::UnparsedPublicKey::new(
            &signature::ECDSA_P256_SHA256_FIXED,
            pair.public_key().as_ref(),
        )
        .verify(input.as_bytes(), &sig)
        .unwrap();
    }

    #[test]
    fn oauth1_twitter_example() {
        // https://developer.x.com/en/docs/authentication/oauth-1-0a/creating-a-signature
        let auth = OAuth1Auth {
            consumer_key: "xvz1evFS4wEEPTGEFPHBog".into(),
            consumer_secret: "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw".into(),
            token: "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb".into(),
            token_secret: "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE".into(),
            ..Default::default()
        };
        let url =
            Url::parse("https://api.twitter.com/1.1/statuses/update.json?include_entities=true")
                .unwrap();
        let form = vec![(
            "status".into(),
            "Hello Ladies + Gentlemen, a signed OAuth request!".into(),
        )];
        let params = oauth1_params(
            &auth,
            "POST",
            &url,
            &form,
            Some(1318622958),
            Some("kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg"),
        );
        let signature = &params
            .iter()
            .find(|(k, _)| k == "oauth_signature")
            .unwrap()
            .1;
        assert_eq!(signature, "hCtSmYh+iHYCEqBWrE7C7hYmtUk=");
        let header = oauth1_header("", &params);
        assert!(
            header.starts_with("OAuth oauth_consumer_key=\"xvz1evFS4wEEPTGEFPHBog\""),
            "{header}"
        );
        assert!(
            header.contains("oauth_signature=\"hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D\""),
            "{header}"
        );
    }
}

// OAuth 2.0: the settings. Getting tokens is `crate::oauth2`'s job; the engine only sends them.

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum OAuth2Grant {
    /// A browser login that returns a code, exchanged for a token (with PKCE when `pkce` is set).
    #[default]
    AuthorizationCode,
    /// A browser login that returns the token directly. Old; prefer the code grant with PKCE.
    Implicit,
    /// The app's own credentials, no user.
    ClientCredentials,
    /// A user name and password sent to the token endpoint.
    Password,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ClientAuthentication {
    /// The client id and secret as HTTP Basic auth.
    #[default]
    Header,
    /// The client id and secret in the form body.
    Body,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct OAuth2Auth {
    pub grant: OAuth2Grant,
    /// The login page, for the browser grants.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub auth_url: String,
    /// Where codes, credentials and refresh tokens are exchanged for tokens.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub access_token_url: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub client_secret: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    /// For the browser grants: a loopback address Tuma listens on, registered with the provider.
    pub redirect_uri: String,
    /// For the code grant: use PKCE (S256).
    pub pkce: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub audience: String,
    pub client_authentication: ClientAuthentication,
    /// Sent as `Authorization: <prefix> <token>`.
    pub prefix: String,
    /// The token Tuma got for these settings; never written to files.
    #[serde(skip)]
    #[schemars(skip)]
    pub access_token: Option<String>,
}

impl Default for OAuth2Auth {
    fn default() -> Self {
        Self {
            grant: OAuth2Grant::AuthorizationCode,
            auth_url: String::new(),
            access_token_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scope: String::new(),
            redirect_uri: "http://localhost:8765/callback".into(),
            pkce: true,
            username: String::new(),
            password: String::new(),
            audience: String::new(),
            client_authentication: ClientAuthentication::Header,
            prefix: "Bearer".into(),
            access_token: None,
        }
    }
}
