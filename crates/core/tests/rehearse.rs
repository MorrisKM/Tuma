//! A rehearsal: the request as it would be sent, after pre-request scripts and auth, with nothing sent or stored.

use std::fs;

use tuma_core::runner::{self, Job};
use tuma_core::secrets::MemoryStore;
use tuma_core::workspace::files::{Auth, RequestFile, Scripts};
use tuma_core::workspace::{VariableValue, Workspace};

#[tokio::test(flavor = "multi_thread")]
async fn rehearsal_applies_scripts_and_auth_but_stores_nothing() {
    let root = std::env::temp_dir().join(format!("tuma-rehearse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let workspace = Workspace::create(&root, "Rehearse").unwrap();
    let secrets = MemoryStore::default();
    workspace
        .write_environment(
            "Local",
            &[VariableValue {
                name: "token".into(),
                value: "old".into(),
                secret: false,
                enabled: true,
            }],
            &secrets,
        )
        .unwrap();
    let request = RequestFile {
        method: "GET".into(),
        // Nothing listens there: a rehearsal must not try.
        url: "http://127.0.0.1:9/items/{{id}}".into(),
        auth: Some(Auth::Bearer {
            token: "{{token}}".into(),
        }),
        scripts: Scripts {
            pre_request: "pm.environment.set('token', 'new'); pm.variables.set('id', '7'); \
                          pm.request.headers.add({ key: 'X-Trace', value: 't1' });"
                .into(),
            test: String::new(),
        },
        ..Default::default()
    };

    let execution = runner::rehearse(Job {
        workspace: Some(&workspace),
        request: &request,
        path: None,
        context: None,
        environment: Some("Local"),
        overrides: &[],
        extra_checks: None,
        secrets: &secrets,
        history: None,
        source: "",
        allow_host: None,
        scripting: Default::default(),
        network: None,
        observer: None,
    })
    .await;

    assert!(
        execution.script_errors.is_empty(),
        "{:?}",
        execution.script_errors
    );
    let sent = execution.sent.expect("the request as it would go");
    assert_eq!(sent.url, "http://127.0.0.1:9/items/7");
    assert!(
        sent.headers
            .iter()
            .any(|h| h.name == "X-Trace" && h.value == "t1")
    );
    assert_eq!(
        sent.auth,
        tuma_core::http::Auth::Bearer {
            token: "new".into()
        }
    );
    assert_eq!(execution.response.unwrap_err().kind, "skipped");
    assert!(
        workspace.current_values().is_empty(),
        "a rehearsal stores no variables"
    );
    assert_eq!(execution.pre_request.len(), 1);
    assert_eq!(execution.pre_request[0].label, "request");
    let curl = tuma_core::codegen::generate(&sent, tuma_core::codegen::Language::Curl);
    assert!(curl.contains("Authorization: Bearer new"), "{curl}");

    // Without auth: neither the Auth tab's nor an Authorization header typed in.
    let mut basic = sent.clone();
    basic.headers.push(tuma_core::http::KeyValue {
        name: "authorization".into(),
        value: "Basic abc".into(),
        enabled: true,
    });
    tuma_core::codegen::strip_auth(&mut basic);
    let curl = tuma_core::codegen::generate(&basic, tuma_core::codegen::Language::Curl);
    assert!(!curl.to_lowercase().contains("authorization"), "{curl}");
    assert!(curl.contains("X-Trace: t1"), "{curl}");

    assert_eq!(
        tuma_core::codegen::comment(
            "Scripts:\n\n  pm.x()",
            tuma_core::codegen::Language::JavaScript
        ),
        "// Scripts:\n//\n//   pm.x()\n"
    );

    let _ = fs::remove_dir_all(&root);
}
