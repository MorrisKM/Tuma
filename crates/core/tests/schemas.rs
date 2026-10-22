//! The JSON Schemas in the repository's `schemas/` folder must match the file types. After changing the on-disk format,
//! regenerate them with `UPDATE_GOLDEN=1 cargo test -p tuma-core --test schemas` and review the diff.

use std::path::Path;

#[test]
fn schemas_match_the_file_types() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    for (name, schema) in tuma_core::check::schemas() {
        let path = dir.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            on_disk, text,
            "schemas/{name}.json is out of date; see this test's comment"
        );
    }
}
