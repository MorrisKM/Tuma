//! `{{name}}` templating. A variable's value may itself use variables. Unknown names are left as they are, like
//! Postman does, and reported so the UI can point them out.

use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::http::auth::OAuth2Auth;
use crate::http::{Auth, Body, KeyValue, Part, Request};

/// Variable scopes, highest priority first. The first scope that defines a name wins.
#[derive(Debug, Clone, Default)]
pub struct Scopes {
    scopes: Vec<HashMap<String, String>>,
}

/// How deep variables may refer to other variables, to stop `{{a}}` = `{{a}}` loops.
const MAX_DEPTH: usize = 10;

impl Scopes {
    /// Adds a scope with lower priority than those already added. Later duplicates within a scope win.
    pub fn push(&mut self, vars: impl IntoIterator<Item = (String, String)>) {
        self.scopes.push(vars.into_iter().collect());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.scopes
            .iter()
            .find_map(|scope| scope.get(name))
            .map(String::as_str)
    }

    /// Replaces every `{{name}}` in `text`. Names that are not defined are kept and added to `unresolved`.
    pub fn resolve(&self, text: &str, unresolved: &mut BTreeSet<String>) -> String {
        self.resolve_depth(text, unresolved, 0)
    }

    fn resolve_depth(&self, text: &str, unresolved: &mut BTreeSet<String>, depth: usize) -> String {
        if !text.contains("{{") {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = after[..end].trim();
            let placeholder = &rest[start..start + 2 + end + 2];
            match self.value(name) {
                Some(value) if depth < MAX_DEPTH => {
                    out.push_str(&self.resolve_depth(&value, unresolved, depth + 1))
                }
                Some(value) => out.push_str(&value),
                None => {
                    unresolved.insert(name.to_string());
                    out.push_str(placeholder);
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn value(&self, name: &str) -> Option<String> {
        if let Some(value) = self.get(name) {
            return Some(value.to_string());
        }
        name.strip_prefix('$').and_then(dynamic)
    }

    /// Resolves every text field of a request that can hold variables.
    pub fn resolve_request(&self, request: &Request) -> (Request, BTreeSet<String>) {
        let mut unresolved = BTreeSet::new();
        let mut u = |s: &str| self.resolve(s, &mut unresolved);
        let rows = |rows: &[KeyValue], u: &mut dyn FnMut(&str) -> String| {
            rows.iter()
                .map(|kv| KeyValue {
                    name: u(&kv.name),
                    value: u(&kv.value),
                    enabled: kv.enabled,
                })
                .collect::<Vec<_>>()
        };

        let url = u(&request.url);
        let headers = rows(&request.headers, &mut u);
        let body = match &request.body {
            Body::None => Body::None,
            Body::Graphql {
                query,
                variables,
                operation_name,
            } => Body::Graphql {
                query: u(query),
                variables: u(variables),
                operation_name: operation_name.as_deref().map(&mut u),
            },
            Body::Raw {
                content,
                content_type,
            } => Body::Raw {
                content: u(content),
                content_type: content_type.clone(),
            },
            Body::FormUrlEncoded { fields } => Body::FormUrlEncoded {
                fields: rows(fields, &mut u),
            },
            Body::Multipart { parts } => Body::Multipart {
                parts: parts
                    .iter()
                    .map(|p| Part {
                        name: u(&p.name),
                        value: u(&p.value),
                        ..p.clone()
                    })
                    .collect(),
            },
            Body::File { path, content_type } => Body::File {
                path: u(&path.to_string_lossy()).into(),
                content_type: content_type.clone(),
            },
        };
        let auth = match &request.auth {
            Auth::Digest(a) => Auth::Digest(resolve_fields(a, &mut u)),
            Auth::Aws(a) => Auth::Aws(resolve_fields(a, &mut u)),
            Auth::Jwt(a) => Auth::Jwt(resolve_fields(a, &mut u)),
            Auth::OAuth1(a) => Auth::OAuth1(resolve_fields(a, &mut u)),
            Auth::OAuth2(a) => Auth::OAuth2(OAuth2Auth {
                access_token: a.access_token.clone(),
                ..resolve_fields(a, &mut u)
            }),
            Auth::RequestToken(a) => Auth::RequestToken(a.clone()),
            Auth::None => Auth::None,
            Auth::Bearer { token } => Auth::Bearer { token: u(token) },
            Auth::Basic { username, password } => Auth::Basic {
                username: u(username),
                password: u(password),
            },
            Auth::ApiKey {
                key,
                value,
                location,
            } => Auth::ApiKey {
                key: u(key),
                value: u(value),
                location: *location,
            },
        };
        let resolved = Request {
            method: request.method.clone(),
            url,
            headers,
            body,
            auth,
            settings: request.settings.clone(),
        };
        (resolved, unresolved)
    }
}

/// Resolves every text field of a settings struct, through its JSON form.
fn resolve_fields<T: serde::Serialize + serde::de::DeserializeOwned + Clone>(
    value: &T,
    u: &mut dyn FnMut(&str) -> String,
) -> T {
    fn walk(v: &mut serde_json::Value, u: &mut dyn FnMut(&str) -> String) {
        match v {
            serde_json::Value::String(s) => *s = u(s),
            serde_json::Value::Array(items) => items.iter_mut().for_each(|i| walk(i, u)),
            serde_json::Value::Object(map) => map.values_mut().for_each(|i| walk(i, u)),
            _ => {}
        }
    }
    let Ok(mut json) = serde_json::to_value(value) else {
        return value.clone();
    };
    walk(&mut json, u);
    serde_json::from_value(json).unwrap_or_else(|_| value.clone())
}

/// Postman's most used dynamic variables: `{{$guid}}`, `{{$timestamp}}` and friends.
fn dynamic(name: &str) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Some(match name {
        "guid" | "randomUUID" => uuid::Uuid::new_v4().to_string(),
        "timestamp" => now.as_secs().to_string(),
        "isoTimestamp" => iso8601(now.as_millis() as i64),
        "randomInt" => (uuid::Uuid::new_v4().as_u128() % 1001).to_string(),
        _ => return None,
    })
}

/// Formats Unix milliseconds as `2026-10-22T09:07:50.123Z`.
fn iso8601(millis: i64) -> String {
    let (days, ms_of_day) = (millis.div_euclid(86_400_000), millis.rem_euclid(86_400_000));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s, ms) = (
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000,
    );
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes() -> Scopes {
        let mut scopes = Scopes::default();
        scopes.push([("host".to_string(), "api.dev".to_string())]);
        scopes.push([
            ("host".to_string(), "api.prod".to_string()),
            ("base".to_string(), "https://{{host}}/v1".to_string()),
            ("loop".to_string(), "{{loop}}".to_string()),
        ]);
        scopes
    }

    #[test]
    fn higher_scopes_win_and_values_nest() {
        let mut unresolved = BTreeSet::new();
        assert_eq!(
            scopes().resolve("{{base}}/users/{{ id }}", &mut unresolved),
            "https://api.dev/v1/users/{{ id }}"
        );
        assert_eq!(unresolved.into_iter().collect::<Vec<_>>(), ["id"]);
    }

    #[test]
    fn loops_stop_and_unclosed_braces_are_kept() {
        let mut unresolved = BTreeSet::new();
        assert_eq!(scopes().resolve("{{loop}}", &mut unresolved), "{{loop}}");
        assert_eq!(scopes().resolve("a {{host", &mut unresolved), "a {{host");
    }

    #[test]
    fn dynamic_variables() {
        let mut unresolved = BTreeSet::new();
        let guid = scopes().resolve("{{$guid}}", &mut unresolved);
        assert_eq!(guid.len(), 36);
        assert!(
            scopes()
                .resolve("{{$timestamp}}", &mut unresolved)
                .parse::<u64>()
                .is_ok()
        );
        assert!(unresolved.is_empty());
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_792_660_070_123), "2026-10-22T09:07:50.123Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn resolves_every_part_of_a_request() {
        let request = Request {
            url: "{{base}}/x".into(),
            headers: vec![KeyValue::new("X-Host", "{{host}}")],
            body: Body::Raw {
                content: "{\"h\":\"{{host}}\"}".into(),
                content_type: "application/json".into(),
            },
            auth: Auth::Bearer {
                token: "{{token}}".into(),
            },
            ..Default::default()
        };
        let (resolved, unresolved) = scopes().resolve_request(&request);
        assert_eq!(resolved.url, "https://api.dev/v1/x");
        assert_eq!(resolved.headers[0].value, "api.dev");
        assert_eq!(
            resolved.body,
            Body::Raw {
                content: "{\"h\":\"api.dev\"}".into(),
                content_type: "application/json".into()
            }
        );
        assert_eq!(
            resolved.auth,
            Auth::Bearer {
                token: "{{token}}".into()
            }
        );
        assert_eq!(unresolved.into_iter().collect::<Vec<_>>(), ["token"]);
    }
}
