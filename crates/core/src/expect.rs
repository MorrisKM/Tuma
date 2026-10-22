//! Checks on a response: the `expect:` block of a request file, and what `tuma send --expect` builds.
//!
//! ```yaml
//! expect:
//!   status: 201          # or a class: 2xx
//!   maxTimeMs: 500
//!   headers:
//!   - name: content-type
//!     contains: json
//!   body:
//!   - path: $.id
//!     exists: true
//!   - path: $.items
//!     length: 3
//!   - path: $.email
//!     matches: "^.+@.+$"
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Response;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Expectations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderCheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<BodyCheck>,
    /// The response must match the OpenAPI spec for its operation and status (a contract check). Needs a collection
    /// imported from a spec.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub schema: bool,
}

impl Expectations {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// An exact status code, or a class such as `2xx`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Status {
    Code(u16),
    Class(String),
}

impl Status {
    fn matches(&self, status: u16) -> bool {
        match self {
            Status::Code(code) => *code == status,
            Status::Class(class) => {
                let class = class.trim().to_ascii_lowercase();
                class.len() == 3
                    && class.ends_with("xx")
                    && class.as_bytes()[0] == b'0' + (status / 100) as u8
            }
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Code(code) => write!(f, "{code}"),
            Status::Class(class) => write!(f, "{class}"),
        }
    }
}

/// A check on a response header. Names are case-insensitive. With no condition, the header must exist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeaderCheck {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    /// `false` checks that the header is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
}

/// A check on the body. `path` is a JSON path such as `$.items[0].id`; without one, the check applies to the whole
/// body as text (`contains`, `matches`) or as JSON (`equals`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BodyCheck {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    /// Any JSON value. `1` and `"1"` are different.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    /// A regular expression the value (as text) must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    /// Number of items in an array or object, or characters in a string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<usize>,
    /// `string`, `number`, `integer`, `boolean`, `array`, `object` or `null`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// The outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    /// What was checked, such as `status is 201` or `$.id exists`.
    pub check: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
}

impl CheckResult {
    fn new(
        check: String,
        passed: bool,
        expected: impl fmt::Display,
        actual: impl fmt::Display,
    ) -> Self {
        Self {
            check,
            passed,
            expected: expected.to_string(),
            actual: actual.to_string(),
        }
    }
}

/// Runs every check against `response`.
pub fn evaluate(expect: &Expectations, response: &Response) -> Vec<CheckResult> {
    let mut results = Vec::new();
    if let Some(status) = &expect.status {
        results.push(CheckResult::new(
            format!("status is {status}"),
            status.matches(response.status),
            status,
            format!("{} {}", response.status, response.reason).trim_end(),
        ));
    }
    if let Some(max) = expect.max_time_ms {
        let ms = response.timing.total.as_secs_f64() * 1000.0;
        results.push(CheckResult::new(
            format!("time under {max} ms"),
            ms <= max as f64,
            format!("<= {max} ms"),
            format!("{ms:.0} ms"),
        ));
    }
    for check in &expect.headers {
        results.push(header_check(check, response));
    }
    if !expect.body.is_empty() {
        let text = String::from_utf8_lossy(&response.body);
        let json: Option<Value> = serde_json::from_str(&text).ok();
        for check in &expect.body {
            results.extend(body_check(check, &text, json.as_ref()));
        }
    }
    results
}

fn header_check(check: &HeaderCheck, response: &Response) -> CheckResult {
    let value = response
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(&check.name))
        .map(|h| h.value.as_str());
    let actual = value
        .map(|v| format!("\"{v}\""))
        .unwrap_or_else(|| "no such header".into());
    let name = &check.name;
    if let Some(expected) = &check.equals {
        return CheckResult::new(
            format!("header {name} is \"{expected}\""),
            value == Some(expected.as_str()),
            format!("\"{expected}\""),
            actual,
        );
    }
    if let Some(part) = &check.contains {
        return CheckResult::new(
            format!("header {name} contains \"{part}\""),
            value.is_some_and(|v| v.contains(part.as_str())),
            format!("contains \"{part}\""),
            actual,
        );
    }
    let want = check.exists.unwrap_or(true);
    let what = if want { "exists" } else { "is absent" };
    CheckResult::new(
        format!("header {name} {what}"),
        value.is_some() == want,
        what,
        actual,
    )
}

fn body_check(check: &BodyCheck, text: &str, json: Option<&Value>) -> Vec<CheckResult> {
    let target = check.path.as_deref().unwrap_or("body");
    // Without a path, text checks look at the raw body.
    let values: Vec<Value> = match (&check.path, json) {
        (Some(path), Some(json)) => match select(json, path) {
            Ok(values) => values.into_iter().cloned().collect(),
            Err(e) => {
                return vec![CheckResult::new(
                    format!("{target} is a valid path"),
                    false,
                    "a JSON path",
                    e,
                )];
            }
        },
        (Some(_), None) => {
            return vec![CheckResult::new(
                format!("{target}: body is JSON"),
                false,
                "a JSON body",
                preview(text),
            )];
        }
        (None, Some(json))
            if check.equals.is_some() || check.length.is_some() || check.kind.is_some() =>
        {
            vec![json.clone()]
        }
        (None, _) => vec![Value::String(text.to_string())],
    };
    // A path with a wildcard can match several values; checks other than `exists` need exactly one.
    let one = match values.as_slice() {
        [value] => Some(value),
        _ => None,
    };
    let actual = || match values.as_slice() {
        [] => "nothing".to_string(),
        [value] => preview(&compact(value)),
        many => format!(
            "{} values: {}",
            many.len(),
            preview(&compact(&Value::Array(many.to_vec())))
        ),
    };

    let mut results = Vec::new();
    let exists_only = check.equals.is_none()
        && check.contains.is_none()
        && check.matches.is_none()
        && check.length.is_none()
        && check.kind.is_none();
    if check.exists.is_some() || exists_only {
        let want = check.exists.unwrap_or(true);
        let what = if want { "exists" } else { "does not exist" };
        results.push(CheckResult::new(
            format!("{target} {what}"),
            values.is_empty() != want,
            what,
            actual(),
        ));
    }
    if let Some(expected) = &check.equals {
        // Lenient on numbers: 1 and 1.0 are equal.
        let passed = one.is_some_and(|v| {
            v == expected || (v.as_f64().is_some() && v.as_f64() == expected.as_f64())
        });
        results.push(CheckResult::new(
            format!("{target} equals {}", compact(expected)),
            passed,
            compact(expected),
            actual(),
        ));
    }
    if let Some(part) = &check.contains {
        let passed = one.is_some_and(|v| match v {
            Value::String(s) => s.contains(part.as_str()),
            Value::Array(items) => items
                .iter()
                .any(|i| i.as_str() == Some(part) || compact(i) == *part),
            other => compact(other).contains(part.as_str()),
        });
        results.push(CheckResult::new(
            format!("{target} contains \"{part}\""),
            passed,
            format!("contains \"{part}\""),
            actual(),
        ));
    }
    if let Some(pattern) = &check.matches {
        let result = match regex::Regex::new(pattern) {
            Ok(re) => {
                let passed = one.is_some_and(|v| re.is_match(&as_text(v)));
                CheckResult::new(
                    format!("{target} matches /{pattern}/"),
                    passed,
                    format!("/{pattern}/"),
                    actual(),
                )
            }
            Err(e) => CheckResult::new(
                format!("{target} matches /{pattern}/"),
                false,
                "a valid regular expression",
                e,
            ),
        };
        results.push(result);
    }
    if let Some(expected) = check.length {
        let length = one.and_then(|v| match v {
            Value::Array(a) => Some(a.len()),
            Value::Object(o) => Some(o.len()),
            Value::String(s) => Some(s.chars().count()),
            _ => None,
        });
        let actual = length.map(|l| l.to_string()).unwrap_or_else(actual);
        results.push(CheckResult::new(
            format!("{target} has length {expected}"),
            length == Some(expected),
            expected,
            actual,
        ));
    }
    if let Some(kind) = &check.kind {
        let actual_kind = one.map(type_name);
        let passed = actual_kind == Some(kind.as_str())
            || (kind == "integer" && one.is_some_and(|v| v.is_i64() || v.is_u64()))
            || (kind == "number" && actual_kind == Some("integer"));
        let actual = actual_kind.map(String::from).unwrap_or_else(actual);
        results.push(CheckResult::new(
            format!("{target} is a {kind}"),
            passed,
            kind,
            actual,
        ));
    }
    results
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => compact(other),
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// The start of a long value, for messages.
fn preview(text: &str) -> String {
    const MAX: usize = 120;
    match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// Evaluates a JSON path: `$`, `.name`, `['name']`, `[0]`, `[-1]` (from the end) and `[*]` / `.*` (every item).
pub fn select<'a>(root: &'a Value, path: &str) -> Result<Vec<&'a Value>, String> {
    let path = path.trim();
    let rest = path
        .strip_prefix('$')
        .ok_or_else(|| format!("\"{path}\" must start with $"))?;
    let mut current = vec![root];
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        let step = match c {
            '.' => {
                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '[' {
                        break;
                    }
                    name.push(c);
                    chars.next();
                }
                if name.is_empty() {
                    return Err(format!("\"{path}\": empty name after ."));
                }
                if name == "*" {
                    Step::All
                } else {
                    Step::Key(name)
                }
            }
            '[' => {
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    inner.push(c);
                }
                let inner = inner.trim();
                if inner == "*" {
                    Step::All
                } else if let Some(quoted) = inner
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .or_else(|| inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
                {
                    Step::Key(quoted.to_string())
                } else {
                    Step::Index(
                        inner
                            .parse()
                            .map_err(|_| format!("\"{path}\": bad index [{inner}]"))?,
                    )
                }
            }
            other => return Err(format!("\"{path}\": unexpected '{other}'")),
        };
        current = current
            .into_iter()
            .flat_map(|value| -> Vec<&Value> {
                match (&step, value) {
                    (Step::Key(key), Value::Object(map)) => map.get(key).into_iter().collect(),
                    (Step::Index(i), Value::Array(items)) => {
                        let index = if *i < 0 { items.len() as i64 + i } else { *i };
                        usize::try_from(index)
                            .ok()
                            .and_then(|i| items.get(i))
                            .into_iter()
                            .collect()
                    }
                    (Step::All, Value::Array(items)) => items.iter().collect(),
                    (Step::All, Value::Object(map)) => map.values().collect(),
                    _ => vec![],
                }
            })
            .collect();
    }
    Ok(current)
}

enum Step {
    Key(String),
    Index(i64),
    All,
}

/// Parses the short form used on the command line:
///
/// - `schema`: the response matches the OpenAPI spec
/// - `status=201`, `status=2xx`
/// - `time<500`
/// - `header content-type~json` (contains), `header x-id` (exists), `header x-id=abc`
/// - `$.id exists`, `$.id !exists`, `$.name == "Ann"` (a JSON value; bare words are strings), `$.email ~ @`
///   (contains), `$.email =~ ^.+@` (regex), `$.items length 3`, `$.id type integer`
pub fn parse_short(text: &str) -> Result<Expectations, String> {
    let text = text.trim();
    let mut expect = Expectations::default();
    let err = || format!("can't read check \"{text}\"; see `tuma send --help` for the forms");
    if text == "schema" {
        expect.schema = true;
    } else if let Some(value) = text
        .strip_prefix("status")
        .map(|s| s.trim_start())
        .and_then(|s| s.strip_prefix('='))
    {
        let value = value.trim().trim_start_matches('=').trim();
        expect.status = Some(
            value
                .parse()
                .map(Status::Code)
                .unwrap_or_else(|_| Status::Class(value.to_string())),
        );
    } else if let Some(value) = text
        .strip_prefix("time")
        .map(str::trim_start)
        .and_then(|s| s.strip_prefix('<'))
    {
        expect.max_time_ms = Some(
            value
                .trim()
                .trim_end_matches("ms")
                .trim()
                .parse()
                .map_err(|_| err())?,
        );
    } else if let Some(rest) = text.strip_prefix("header ") {
        let rest = rest.trim();
        let check = if let Some((name, part)) = rest.split_once('~') {
            HeaderCheck {
                name: name.trim().into(),
                contains: Some(part.trim().into()),
                ..Default::default()
            }
        } else if let Some((name, value)) = rest.split_once('=') {
            HeaderCheck {
                name: name.trim().into(),
                equals: Some(value.trim().into()),
                ..Default::default()
            }
        } else if let Some(name) = rest.strip_prefix('!') {
            HeaderCheck {
                name: name.trim().into(),
                exists: Some(false),
                ..Default::default()
            }
        } else {
            HeaderCheck {
                name: rest.into(),
                ..Default::default()
            }
        };
        expect.headers.push(check);
    } else if text.starts_with('$') {
        let (path, rest) = text
            .split_once(char::is_whitespace)
            .unwrap_or((text, "exists"));
        let rest = rest.trim();
        let path = Some(path.to_string());
        let check = if rest == "exists" {
            BodyCheck {
                path,
                exists: Some(true),
                ..Default::default()
            }
        } else if rest == "!exists" {
            BodyCheck {
                path,
                exists: Some(false),
                ..Default::default()
            }
        } else if let Some(value) = rest.strip_prefix("==") {
            let value = value.trim();
            let equals =
                serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));
            BodyCheck {
                path,
                equals: Some(equals),
                ..Default::default()
            }
        } else if let Some(pattern) = rest.strip_prefix("=~") {
            BodyCheck {
                path,
                matches: Some(pattern.trim().into()),
                ..Default::default()
            }
        } else if let Some(part) = rest.strip_prefix('~') {
            BodyCheck {
                path,
                contains: Some(part.trim().into()),
                ..Default::default()
            }
        } else if let Some(n) = rest.strip_prefix("length") {
            BodyCheck {
                path,
                length: Some(n.trim().parse().map_err(|_| err())?),
                ..Default::default()
            }
        } else if let Some(kind) = rest.strip_prefix("type") {
            BodyCheck {
                path,
                kind: Some(kind.trim().into()),
                ..Default::default()
            }
        } else {
            return Err(err());
        };
        expect.body.push(check);
    } else {
        return Err(err());
    }
    Ok(expect)
}

impl Expectations {
    /// Adds the checks of `other` to these. A status or time limit in `other` replaces this one's.
    pub fn merge(&mut self, other: Expectations) {
        if other.status.is_some() {
            self.status = other.status;
        }
        if other.max_time_ms.is_some() {
            self.max_time_ms = other.max_time_ms;
        }
        self.headers.extend(other.headers);
        self.body.extend(other.body);
        self.schema |= other.schema;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Header, SentRequest, Timing};
    use serde_json::json;

    fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            reason: "Created".into(),
            version: "HTTP/1.1".into(),
            url: "http://x".into(),
            remote_addr: None,
            headers: vec![Header {
                name: "Content-Type".into(),
                value: "application/json; charset=utf-8".into(),
            }],
            body: body.to_string().into(),
            encoded_size: body.len(),
            timing: Timing::default(),
            redirects: vec![],
            sent: SentRequest {
                method: "GET".into(),
                url: "http://x".into(),
                headers: vec![],
                body_size: 0,
            },
        }
    }

    fn checks(lines: &[&str]) -> Expectations {
        let mut expect = Expectations::default();
        for line in lines {
            expect.merge(parse_short(line).unwrap());
        }
        expect
    }

    #[test]
    fn json_paths() {
        let doc = json!({"items": [{"id": 1}, {"id": 2}], "a b": true});
        let ids: Vec<_> = select(&doc, "$.items[*].id").unwrap();
        assert_eq!(ids, [&json!(1), &json!(2)]);
        assert_eq!(select(&doc, "$.items[-1].id").unwrap(), [&json!(2)]);
        assert_eq!(select(&doc, "$['a b']").unwrap(), [&json!(true)]);
        assert!(select(&doc, "$.nope.deeper").unwrap().is_empty());
        assert!(select(&doc, "items").is_err());
    }

    #[test]
    fn passing_and_failing_checks() {
        let r = response(
            201,
            r#"{"id": 7, "name": "Ann", "email": "ann@x.io", "tags": ["a", "b"]}"#,
        );
        let expect = checks(&[
            "status=2xx",
            "header content-type~json",
            "$.id exists",
            "$.id type integer",
            r#"$.name == "Ann""#,
            "$.name == Ann",
            "$.email =~ ^[^@]+@x\\.io$",
            "$.tags length 2",
            "$.tags ~ b",
            "$.missing !exists",
        ]);
        let results = evaluate(&expect, &r);
        assert!(results.iter().all(|r| r.passed), "{results:#?}");

        let results = evaluate(
            &checks(&["status=200", "$.id == 8", "$.nope exists", "header x-id"]),
            &r,
        );
        assert!(results.iter().all(|r| !r.passed));
        // Status first, then headers, then body checks.
        let actual: Vec<_> = results.iter().map(|r| r.actual.as_str()).collect();
        assert_eq!(actual, ["201 Created", "no such header", "7", "nothing"]);
    }

    #[test]
    fn text_bodies() {
        let r = response(200, "plain hello");
        assert!(
            evaluate(&checks(&["$.x exists"]), &r)
                .iter()
                .all(|c| !c.passed)
        );
        let body = Expectations {
            body: vec![BodyCheck {
                contains: Some("hello".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(evaluate(&body, &r)[0].passed);
    }

    #[test]
    fn yaml_form_round_trips() {
        let expect = checks(&["status=201", "time<500", "$.id exists"]);
        let yaml = crate::workspace::files::to_yaml(&expect).unwrap();
        assert_eq!(
            yaml,
            "status: 201\nmaxTimeMs: 500\nbody:\n- path: $.id\n  exists: true\n"
        );
        assert_eq!(
            serde_saphyr::from_str::<Expectations>(&yaml).unwrap(),
            expect
        );
    }
}
