//! Workspace storage, variable resolution and the Postman importer, against real files in a temporary folder.
//!
//! The import test compares the written workspace with `fixtures/postman/expected/`. After an intended format change,
//! regenerate it with `UPDATE_GOLDEN=1 cargo test -p tuma-core --test workspace` and review the diff.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use tuma_core::http::{ApiKeyLocation, Auth};
use tuma_core::import::{self, Imported};
use tuma_core::secrets::{MemoryStore, SecretStore};
use tuma_core::workspace::files::RequestFile;
use tuma_core::workspace::{NodeKind, VariableValue, Workspace};

/// A fresh, empty folder that is removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tuma-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman")
            .join(name),
    )
    .unwrap()
}

/// Every file under `dir` by relative path, except workspace.yaml whose id is random.
fn files(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.file_name().unwrap() != "workspace.yaml" {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn imported_workspace(dir: &TempDir, secrets: &MemoryStore) -> Workspace {
    let mut workspace = Workspace::create(&dir.0, "Test").unwrap();
    let (imported, _) =
        import::postman(&mut workspace, &fixture("collection.json"), secrets).unwrap();
    assert_eq!(imported, Imported::Collection("Shop API".into()));
    let (imported, _) =
        import::postman(&mut workspace, &fixture("environment.json"), secrets).unwrap();
    assert_eq!(imported, Imported::Environment("Staging".into()));
    workspace
}

#[test]
fn postman_import_matches_golden_files() {
    let dir = TempDir::new("golden");
    let secrets = MemoryStore::default();
    let mut workspace = Workspace::create(&dir.0, "Test").unwrap();
    let (_, warnings) =
        import::postman(&mut workspace, &fixture("collection.json"), &secrets).unwrap();
    import::postman(&mut workspace, &fixture("environment.json"), &secrets).unwrap();

    assert_eq!(
        warnings.0,
        ["\"List products\": 1 disabled query parameter(s) dropped",]
    );

    // What the importer writes is valid by `tuma check`'s strict rules.
    assert_eq!(tuma_core::check::check_workspace(&dir.0), vec![]);

    let expected_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/postman/expected");
    let actual = files(&dir.0);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let _ = fs::remove_dir_all(&expected_dir);
        for (path, content) in &actual {
            let target = expected_dir.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
    }
    assert_eq!(actual, files(&expected_dir));
}

#[test]
fn imported_tree_keeps_postman_order() {
    let dir = TempDir::new("tree");
    let workspace = imported_workspace(&dir, &MemoryStore::default());

    let tree = workspace.tree().unwrap();
    assert_eq!(tree.len(), 1);
    let names: Vec<_> = tree[0]
        .children
        .iter()
        .map(|n| (n.kind, n.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (NodeKind::Request, "Health"),
            (NodeKind::Folder, "Products"),
            (NodeKind::Folder, "Orders")
        ]
    );
    let orders: Vec<_> = tree[0].children[2]
        .children
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(
        orders,
        [
            "Create order",
            "Upload receipt",
            "Login- form - urlencoded",
            "Search",
            "Signed"
        ]
    );
    assert_eq!(
        tree[0].children[2].children[0].method.as_deref(),
        Some("POST")
    );
}

#[test]
fn prepare_inherits_auth_and_resolves_variables() {
    let dir = TempDir::new("prepare");
    let secrets = MemoryStore::default();
    let workspace = imported_workspace(&dir, &secrets);
    let prepare = |path: &str, env: Option<&str>| {
        let file = workspace.read_request(path).unwrap();
        workspace
            .prepare(&file, Some(path), env, &[], &secrets)
            .unwrap()
    };

    // Inherits the collection's bearer auth; the token is a secret in the environment.
    let health = prepare("Shop API/Health.yaml", Some("Staging"));
    assert_eq!(health.request.url, "https://staging.example.com/health");
    assert_eq!(
        health.request.auth,
        Auth::Bearer {
            token: "staging-token".into()
        }
    );
    assert!(health.unresolved.is_empty());

    // Without an environment, the collection variable applies and the token is unresolved.
    let health = prepare("Shop API/Health.yaml", None);
    assert_eq!(health.request.url, "http://localhost:3000/health");
    assert_eq!(health.unresolved.into_iter().collect::<Vec<_>>(), ["token"]);

    // The Products folder says "no auth", which beats the collection's bearer.
    let product = prepare("Shop API/Products/Get product.yaml", Some("Staging"));
    assert_eq!(
        product.request.url,
        "https://staging.example.com/products/42"
    );
    assert_eq!(product.request.auth, Auth::None);

    // The request's own auth wins.
    let order = prepare("Shop API/Orders/Create order.yaml", Some("Staging"));
    assert_eq!(
        order.request.auth,
        Auth::ApiKey {
            key: "X-Api-Key".into(),
            value: "{{apiKey}}".into(),
            location: ApiKeyLocation::Header
        }
    );
    assert_eq!(order.unresolved.into_iter().collect::<Vec<_>>(), ["apiKey"]);
}

#[test]
fn folder_headers_and_variables_apply_nearest_first() {
    let dir = TempDir::new("folders");
    let secrets = MemoryStore::default();
    let workspace = Workspace::create(&dir.0, "Test").unwrap();
    let collection = workspace.create_folder(None, "API").unwrap();
    let folder = workspace.create_folder(Some(&collection), "Admin").unwrap();
    let mut settings = workspace.read_folder(&collection).unwrap();
    settings.headers = vec![tuma_core::workspace::files::KeyValue {
        name: "X-Team".into(),
        value: "{{team}}".into(),
        enabled: true,
    }];
    settings.variables = vec![tuma_core::workspace::files::Variable::new("team", "web")];
    workspace.write_folder(&collection, &settings).unwrap();
    let mut settings = workspace.read_folder(&folder).unwrap();
    settings.variables = vec![tuma_core::workspace::files::Variable::new("team", "admins")];
    workspace.write_folder(&folder, &settings).unwrap();
    let file = RequestFile {
        url: "example.com".into(),
        ..Default::default()
    };
    let path = workspace.create_request(&folder, "Me", &file).unwrap();

    let prepared = workspace
        .prepare(&file, Some(&path), None, &[], &secrets)
        .unwrap();
    assert_eq!(prepared.request.headers.len(), 1);
    assert_eq!(prepared.request.headers[0].value, "admins");
}

#[test]
fn create_rename_delete_keep_the_order() {
    let dir = TempDir::new("crud");
    let workspace = Workspace::create(&dir.0, "Test").unwrap();
    let collection = workspace.create_folder(None, "API").unwrap();
    let request = RequestFile::default();
    let b = workspace
        .create_request(&collection, "b", &request)
        .unwrap();
    workspace
        .create_request(&collection, "a", &request)
        .unwrap();
    let again = workspace
        .create_request(&collection, "b", &request)
        .unwrap();
    assert_eq!(again, "API/b (2).yaml");

    let names = |w: &Workspace| {
        w.tree().unwrap()[0]
            .children
            .iter()
            .map(|n| n.name.clone())
            .collect::<Vec<_>>()
    };
    // Created order, not alphabetical.
    assert_eq!(names(&workspace), ["b", "a", "b (2)"]);

    let renamed = workspace.rename(&b, "z: first?").unwrap();
    assert_eq!(renamed, "API/z- first-.yaml");
    assert_eq!(names(&workspace), ["z- first-", "a", "b (2)"]);
    assert!(workspace.rename("API/a.yaml", "b (2)").is_err());

    workspace.delete("API/a.yaml").unwrap();
    assert_eq!(names(&workspace), ["z- first-", "b (2)"]);
    workspace.delete(&collection).unwrap();
    assert!(workspace.tree().unwrap().is_empty());
}

#[test]
fn duplicate_copies_next_to_the_original() {
    let dir = TempDir::new("duplicate");
    let workspace = Workspace::create(&dir.0, "Test").unwrap();
    let collection = workspace.create_folder(None, "API").unwrap();
    let folder = workspace.create_folder(Some(&collection), "Users").unwrap();
    let request = RequestFile {
        url: "https://example.com/users".into(),
        ..RequestFile::default()
    };
    let list = workspace.create_request(&folder, "list", &request).unwrap();
    workspace
        .create_request(&collection, "health", &request)
        .unwrap();

    // A request: right after the original, with the same contents.
    let copy = workspace.duplicate(&list).unwrap();
    assert_eq!(copy, "API/Users/list copy.yaml");
    assert_eq!(workspace.read_request(&copy).unwrap(), request);
    assert_eq!(
        workspace.duplicate(&list).unwrap(),
        "API/Users/list copy (2).yaml"
    );

    // A folder, with everything in it, also right after the original.
    let folder_copy = workspace.duplicate(&folder).unwrap();
    assert_eq!(folder_copy, "API/Users copy");
    let names: Vec<_> = workspace.tree().unwrap()[0]
        .children
        .iter()
        .map(|n| n.name.clone())
        .collect();
    assert_eq!(names, ["Users", "Users copy", "health"]);
    let inside: Vec<_> = workspace.tree().unwrap()[0].children[1]
        .children
        .iter()
        .map(|n| n.name.clone())
        .collect();
    // Each copy goes right after its original, so the newest copy comes first.
    assert_eq!(inside, ["list", "list copy (2)", "list copy"]);

    // A whole collection.
    assert_eq!(workspace.duplicate(&collection).unwrap(), "API copy");
    assert_eq!(workspace.tree().unwrap().len(), 2);
}

#[test]
fn paths_cannot_leave_the_workspace() {
    let dir = TempDir::new("paths");
    let workspace = Workspace::create(&dir.0, "Test").unwrap();
    for path in ["../x.yaml", "/etc/passwd", "a/../../x.yaml", ""] {
        assert!(workspace.read_request(path).is_err(), "{path}");
        assert!(workspace.delete(path).is_err(), "{path}");
    }
}

#[test]
fn secrets_stay_out_of_files() {
    let dir = TempDir::new("secrets");
    let secrets = MemoryStore::default();
    let mut workspace = Workspace::create(&dir.0, "Test").unwrap();
    let env = workspace.create_environment("Prod").unwrap();
    let vars = vec![
        VariableValue {
            name: "url".into(),
            value: "https://api".into(),
            secret: false,
            enabled: true,
        },
        VariableValue {
            name: "key".into(),
            value: "s3cret".into(),
            secret: true,
            enabled: true,
        },
    ];
    workspace.write_environment(&env, &vars, &secrets).unwrap();
    workspace.write_globals(&vars, &secrets).unwrap();

    let on_disk = files(&dir.0).into_values().collect::<String>()
        + &fs::read_to_string(dir.0.join("workspace.yaml")).unwrap();
    assert!(!on_disk.contains("s3cret"));
    assert_eq!(workspace.read_environment(&env, &secrets).unwrap(), vars);
    assert_eq!(workspace.read_globals(&secrets).unwrap(), vars);

    // Renaming moves the secret; making it non-secret or deleting the environment removes it from the keychain.
    let env = workspace
        .rename_environment(&env, "Production", &secrets)
        .unwrap();
    assert_eq!(workspace.read_environment(&env, &secrets).unwrap(), vars);
    workspace.delete_environment(&env, &secrets).unwrap();
    assert!(workspace.environments().unwrap().is_empty());
    let id = fs::read_to_string(dir.0.join("workspace.yaml")).unwrap();
    let id = id
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .to_string();
    assert_eq!(
        secrets
            .get(&format!("{id}/environments/Production/key"))
            .unwrap(),
        None
    );
    assert_eq!(
        secrets
            .get(&format!("{id}/globals/key"))
            .unwrap()
            .as_deref(),
        Some("s3cret")
    );
}

#[test]
fn postman_export_round_trips() {
    let dir = TempDir::new("export");
    let secrets = MemoryStore::default();
    let workspace = imported_workspace(&dir, &secrets);
    let original = workspace.read_tree("Shop API").unwrap();

    let exported = tuma_core::export::postman::collection(&original).to_string();
    let mut warnings = tuma_core::import::Warnings::default();
    let tuma_core::import::postman::Export::Collection(again) =
        tuma_core::import::postman::parse(&exported, &mut warnings).unwrap()
    else {
        panic!("not a collection");
    };
    assert!(warnings.0.is_empty(), "{warnings:?}");
    // Order lists are rebuilt on write; everything else survives.
    let strip = |mut tree: tuma_core::workspace::FolderTree| {
        fn walk(tree: &mut tuma_core::workspace::FolderTree) {
            tree.folder.order.clear();
            for item in &mut tree.items {
                if let tuma_core::workspace::TreeItem::Folder(f) = item {
                    walk(f);
                }
            }
        }
        walk(&mut tree);
        tree
    };
    assert_eq!(strip(*again), strip(original));
}

#[test]
fn variables_in_scope_say_where_they_come_from() {
    let dir = TempDir::new("scope");
    let secrets = MemoryStore::default();
    let workspace = imported_workspace(&dir, &secrets);
    let vars = workspace
        .variables_in_scope(
            Some("Shop API/Orders/Create order.yaml"),
            Some("Staging"),
            &secrets,
        )
        .unwrap();
    let summary: Vec<(&str, &str, bool)> = vars
        .iter()
        .map(|v| (v.name.as_str(), v.source.as_str(), v.secret))
        .collect();
    assert_eq!(
        summary,
        [
            ("baseUrl", "environment Staging", false),
            ("token", "environment Staging", true),
            ("retries", "collection Shop API", false),
        ]
    );
}

#[test]
fn workspaces_move_by_folder_and_zip() {
    use tuma_core::workspace::archive;
    let dir = TempDir::new("archive");
    let source = dir.0.join("source");
    let workspace = Workspace::create(&source, "Shop API").unwrap();
    let collection = workspace.create_folder(None, "Orders").unwrap();
    workspace
        .create_request(&collection, "list", &RequestFile::default())
        .unwrap();
    fs::create_dir_all(source.join(".git")).unwrap();
    fs::write(source.join(".git/config"), "x").unwrap();
    let into = dir.0.join("workspaces");

    // From a folder: copied under the workspace's name, without hidden files.
    let copied = archive::import(&source, &into).unwrap();
    assert_eq!(copied, into.join("Shop API"));
    assert!(copied.join("Orders/list.yaml").is_file());
    assert!(!copied.join(".git").exists());
    // Again: a numbered folder, and the original is untouched.
    assert_eq!(
        archive::import(&source, &into).unwrap(),
        into.join("Shop API (2)")
    );
    assert!(source.join(".git/config").is_file());

    // Through a .zip.
    let zip = dir.0.join("shop.zip");
    assert_eq!(archive::export(&workspace, &zip).unwrap(), 3);
    let unzipped = archive::import(&zip, &into).unwrap();
    assert_eq!(unzipped, into.join("Shop API (3)"));
    let reopened = Workspace::open(&unzipped).unwrap();
    assert_eq!(reopened.name(), "Shop API");
    assert_eq!(reopened.tree().unwrap()[0].children[0].name, "list");

    // Anything else is refused.
    assert!(archive::import(&dir.0.join("workspaces"), &dir.0.join("elsewhere")).is_err());
}

#[test]
fn imports_insomnia_bruno_and_plain_json() {
    let dir = TempDir::new("json-imports");
    let secrets = MemoryStore::default();
    let mut workspace = Workspace::create(&dir.0, "Test").unwrap();

    // Insomnia: a workspace with a folder, a request using {{ _.var }}, and a base and sub environment.
    let insomnia = r#"{"_type":"export","__export_format":4,"resources":[
      {"_id":"wrk_1","_type":"workspace","name":"Shop"},
      {"_id":"fld_1","_type":"request_group","parentId":"wrk_1","name":"Orders","environment":{"limit":"10"}},
      {"_id":"req_1","_type":"request","parentId":"fld_1","name":"List orders","method":"get",
       "url":"{{ _.baseUrl }}/orders","parameters":[{"name":"page","value":"1"}],
       "headers":[{"name":"Accept","value":"application/json"},{"name":"X-Off","value":"1","disabled":true}],
       "authentication":{"type":"bearer","token":"{{ _.token }}"},
       "body":{"mimeType":"application/json","text":"{\"a\":1}"}},
      {"_id":"env_1","_type":"environment","parentId":"wrk_1","name":"Base","data":{"baseUrl":"http://localhost"}},
      {"_id":"env_2","_type":"environment","parentId":"env_1","name":"Staging","data":{"baseUrl":"https://staging","token":"t"}}
    ]}"#;
    let (imported, _) = import::any(&mut workspace, insomnia, "insomnia.json", &secrets).unwrap();
    assert_eq!(
        imported,
        Imported::Several(vec![
            Imported::Collection("Shop".into()),
            Imported::Environment("Staging".into())
        ])
    );
    let request = workspace
        .read_request("Shop/Orders/List orders.yaml")
        .unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "{{baseUrl}}/orders?page=1");
    assert!(!request.headers[1].enabled);
    assert!(
        matches!(&request.auth, Some(tuma_core::workspace::files::Auth::Bearer { token }) if token == "{{token}}")
    );
    assert_eq!(
        workspace.read_folder("Shop/Orders").unwrap().variables[0].name,
        "limit"
    );
    let staging = workspace.read_environment("Staging", &secrets).unwrap();
    assert!(
        staging
            .iter()
            .any(|v| v.name == "baseUrl" && v.value == "https://staging")
    );

    // Bruno: requests sorted by seq, path params filled in, scripts kept as comments, environments.
    let bruno = r#"{"name":"Pets","version":"1","items":[
      {"type":"http-request","name":"Second","seq":2,"request":{"url":"{{host}}/pets","method":"POST",
        "headers":[],"body":{"mode":"json","json":"{\"name\":\"Rex\"}"},"auth":{"mode":"inherit"}}},
      {"type":"folder","name":"Admin","items":[
        {"type":"http-request","name":"Get","seq":1,"request":{"url":"{{host}}/pets/:id","method":"GET",
          "params":[{"name":"id","value":"7","type":"path","enabled":true}],
          "script":{"req":"bru.setVar('x', 1)"},"body":{"mode":"none"},"auth":{"mode":"none"}}}]},
      {"type":"http-request","name":"First","seq":1,"request":{"url":"{{host}}/health","method":"GET","body":{"mode":"none"}}}
    ],"environments":[{"name":"Local","variables":[{"name":"host","value":"http://localhost:3000","enabled":true}]}]}"#;
    let (imported, warnings) = import::any(&mut workspace, bruno, "pets.json", &secrets).unwrap();
    assert_eq!(
        imported,
        Imported::Several(vec![
            Imported::Collection("Pets".into()),
            Imported::Environment("Local".into())
        ])
    );
    let names: Vec<_> = workspace
        .tree()
        .unwrap()
        .into_iter()
        .find(|n| n.name == "Pets")
        .unwrap()
        .children
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(names, ["Admin", "First", "Second"]);
    let get = workspace.read_request("Pets/Admin/Get.yaml").unwrap();
    assert_eq!(get.url, "{{host}}/pets/7");
    assert!(get.scripts.pre_request.contains("// bru.setVar('x', 1)"));
    assert!(warnings.0.iter().any(|w| w.contains("Bruno")));

    // A plain JSON object is an environment named after the file.
    let (imported, _) = import::any(
        &mut workspace,
        r#"{"baseUrl": "https://api.example.com", "retries": 3}"#,
        "/x/Production.json",
        &secrets,
    )
    .unwrap();
    assert_eq!(imported, Imported::Environment("Production".into()));
    let production = workspace.read_environment("Production", &secrets).unwrap();
    assert_eq!(production[1].value, "3");
}
