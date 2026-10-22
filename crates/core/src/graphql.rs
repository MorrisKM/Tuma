//! GraphQL schema introspection: the request's URL, headers and auth, with the standard introspection query.

use serde_json::Value;

use crate::workspace::files::{Body, RequestFile};

/// The query graphql-js's `getIntrospectionQuery()` builds, with descriptions, so the result feeds
/// `buildClientSchema` in the app.
pub const INTROSPECTION_QUERY: &str = r#"query IntrospectionQuery {
  __schema {
    queryType { name }
    mutationType { name }
    subscriptionType { name }
    types { ...FullType }
    directives { name description locations args { ...InputValue } }
  }
}
fragment FullType on __Type {
  kind name description
  fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason }
  inputFields { ...InputValue }
  interfaces { ...TypeRef }
  enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason }
  possibleTypes { ...TypeRef }
}
fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue }
fragment TypeRef on __Type {
  kind name
  ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } }
}"#;

/// The introspection request for a GraphQL request: same URL, headers and auth, POST, and no scripts' body.
pub fn introspection_request(request: &RequestFile) -> RequestFile {
    RequestFile {
        method: "POST".into(),
        body: Body::Graphql {
            query: INTROSPECTION_QUERY.into(),
            variables: String::new(),
            operation_name: Some("IntrospectionQuery".into()),
        },
        expect: Default::default(),
        scripts: Default::default(),
        ..request.clone()
    }
}

/// The `data` of an introspection response (`{"__schema": ...}`), or why there is none.
pub fn schema_from_response(status: u16, body: &[u8]) -> Result<Value, String> {
    let json: Value = serde_json::from_slice(body)
        .map_err(|_| format!("the server answered {status} with something that is not JSON"))?;
    if let Some(errors) = json["errors"].as_array().filter(|e| !e.is_empty()) {
        let messages: Vec<&str> = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect();
        return Err(format!(
            "the server refused introspection: {}",
            if messages.is_empty() {
                "unknown error".to_string()
            } else {
                messages.join("; ")
            }
        ));
    }
    let data = json.get("data").filter(|d| d.get("__schema").is_some());
    data.cloned()
        .ok_or_else(|| format!("the server answered {status} without a schema"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http;

    #[test]
    fn graphql_bodies_are_json() {
        assert_eq!(
            http::Body::graphql_json("{ me { id } }", r#"{"id": 1}"#, Some("Me")).unwrap(),
            r#"{"query":"{ me { id } }","variables":{"id":1},"operationName":"Me"}"#
        );
        assert_eq!(
            http::Body::graphql_json("{ a }", "  ", None).unwrap(),
            r#"{"query":"{ a }"}"#
        );
        assert!(
            http::Body::graphql_json("{ a }", "{", None)
                .unwrap_err()
                .contains("not valid JSON")
        );
    }

    #[test]
    fn reads_introspection_answers() {
        let ok = br#"{"data": {"__schema": {"types": []}}}"#;
        assert_eq!(
            schema_from_response(200, ok).unwrap()["__schema"]["types"],
            serde_json::json!([])
        );
        let refused = br#"{"errors": [{"message": "introspection is disabled"}]}"#;
        assert!(
            schema_from_response(400, refused)
                .unwrap_err()
                .contains("introspection is disabled")
        );
        assert!(
            schema_from_response(404, b"<html>")
                .unwrap_err()
                .contains("404")
        );
    }
}
