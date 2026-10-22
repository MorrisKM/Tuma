//! API docs generated from a workspace: OpenAPI that Tuma's own importer reads back, Markdown and HTML.

use std::fs;
use std::path::PathBuf;

use tuma_core::docs::{self, Format};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::files::{Auth, Body, Example, FolderFile, KeyValue, RequestFile};
use tuma_core::workspace::{VariableValue, Workspace};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn workspace(name: &str, secrets: &MemoryStore) -> (TempDir, Workspace) {
    let dir =
        TempDir(std::env::temp_dir().join(format!("tuma-docs-{name}-{}", std::process::id())));
    let _ = fs::remove_dir_all(&dir.0);
    let w = Workspace::create(&dir.0, "Shop").unwrap();
    let api = w.create_folder(None, "Shop API").unwrap();
    w.write_folder(
        &api,
        &FolderFile {
            description: "Orders and products.".into(),
            auth: Some(Auth::Bearer {
                token: "{{token}}".into(),
            }),
            ..FolderFile::default()
        },
    )
    .unwrap();
    let orders = w.create_folder(Some(&api), "Orders").unwrap();
    w.create_request(&orders, "Get order", &RequestFile {
        url: "{{baseUrl}}/orders/:id?expand=items".into(),
        description: "One order by id.".into(),
        headers: vec![KeyValue { name: "X-Api-Key".into(), value: "live-123".into(), enabled: true }],
        examples: vec![
            Example {
                name: "Found".into(),
                status: 200,
                headers: vec![KeyValue { name: "Content-Type".into(), value: "application/json".into(), enabled: true }],
                body: r#"{"id": 7, "email": "ada@example.com", "items": [{"sku": "A1", "qty": 2}]}"#.into(),
            },
            Example { name: "Missing".into(), status: 404, ..Example::default() },
        ],
        ..RequestFile::default()
    })
    .unwrap();
    w.create_request(
        &orders,
        "Create order",
        &RequestFile {
            method: "POST".into(),
            url: "{{baseUrl}}/orders".into(),
            // A comment, which the docs leave out.
            body: Body::Json {
                content: "{\n  // the product\n  \"sku\": \"A1\",\n  \"qty\": 2\n}".into(),
            },
            auth: Some(Auth::None),
            ..RequestFile::default()
        },
    )
    .unwrap();
    w.create_request(
        &api,
        "Live prices",
        &RequestFile {
            url: "wss://shop.example.com/prices".into(),
            ..RequestFile::default()
        },
    )
    .unwrap();
    let env = w.create_environment("Production").unwrap();
    w.write_environment(
        &env,
        &[
            VariableValue {
                name: "baseUrl".into(),
                value: "https://api.shop.example".into(),
                secret: false,
                enabled: true,
            },
            VariableValue {
                name: "token".into(),
                value: "very-secret".into(),
                secret: true,
                enabled: true,
            },
        ],
        secrets,
    )
    .unwrap();
    (dir, w)
}

#[test]
fn openapi_reads_back_into_the_same_requests() {
    let secrets = MemoryStore::default();
    let (_dir, w) = workspace("openapi", &secrets);
    let generated =
        docs::generate(&w, None, Some("Production"), Format::OpenApiYaml, &secrets).unwrap();
    let text = &generated.text;
    assert!(text.contains("openapi: \"3.1.0\""), "{text}");
    assert!(text.contains("https://api.shop.example"));
    assert!(text.contains("/orders/{id}"));
    assert!(
        !text.contains("very-secret") && !text.contains("live-123"),
        "secrets leaked:\n{text}"
    );
    assert!(generated.warnings.iter().any(|w| w.contains("Live prices")));

    let json = docs::generate(&w, None, Some("Production"), Format::OpenApiJson, &secrets)
        .unwrap()
        .text;
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    let get = &spec["paths"]["/orders/{id}"]["get"];
    assert_eq!(get["parameters"][0]["in"], "path");
    assert_eq!(
        get["responses"]["200"]["content"]["application/json"]["schema"]["properties"]["email"]["format"],
        "email"
    );
    assert_eq!(get["security"][0]["bearerAuth"], serde_json::json!([]));
    assert_eq!(
        spec["paths"]["/orders"]["post"]["security"],
        serde_json::json!([])
    );
    assert_eq!(
        spec["paths"]["/orders"]["post"]["requestBody"]["content"]["application/json"]["example"]["qty"],
        2
    );
    assert_eq!(
        spec["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );

    // Tuma's importer understands it: the same operations come back.
    let parsed = tuma_core::openapi::Spec::parse(text).unwrap();
    let mut warnings = Default::default();
    let tree = parsed.to_tree(None, &mut warnings);
    let mut names = Vec::new();
    fn walk(tree: &tuma_core::workspace::FolderTree, out: &mut Vec<String>) {
        for item in &tree.items {
            match item {
                tuma_core::workspace::TreeItem::Request { file, .. } => {
                    out.push(format!("{} {}", file.method, file.url))
                }
                tuma_core::workspace::TreeItem::Folder(f) => walk(f, out),
            }
        }
    }
    walk(&tree, &mut names);
    names.sort();
    // The importer leaves optional query parameters out of the URL.
    assert_eq!(
        names,
        ["GET {{baseUrl}}/orders/{{id}}", "POST {{baseUrl}}/orders"]
    );
}

#[test]
fn markdown_and_html_describe_every_request() {
    let secrets = MemoryStore::default();
    let (_dir, w) = workspace("md", &secrets);
    let md = docs::generate(&w, Some("Shop API"), None, Format::Markdown, &secrets)
        .unwrap()
        .text;
    assert!(md.starts_with("# Shop API\n"));
    assert!(md.contains("### Get order") && md.contains("`GET /orders/{id}`"));
    assert!(md.contains("### Live prices") && md.contains("`WS /prices`"));
    assert!(md.contains("**Auth:** Bearer token"));
    assert!(!md.contains("// the product"), "comments are left out");
    assert!(!md.contains("live-123"));

    let html = docs::generate(
        &w,
        Some("Shop API/Orders/Get order.yaml"),
        None,
        Format::Html,
        &secrets,
    )
    .unwrap()
    .text;
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<h3>Get order</h3>"));
    assert!(!html.contains("Create order"), "only the request asked for");
    assert!(html.contains("ada@example.com"));
}
