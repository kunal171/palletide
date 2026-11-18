//! Exercise the CLI's immutable migration files and schema snapshots.
use std::{path::Path, process::Command};
fn run(schema: &Path, out: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_palletide-codegen"))
        .args([
            "generate",
            schema.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}
#[test]
fn schema_changes_append_migrations_and_never_rewrite_applied_sql() {
    let root =
        std::env::temp_dir().join(format!("palletide-cli-migrations-{}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    let schema = root.join("schema.graphql");
    let out = root.join("generated");
    std::fs::write(&schema, "type Account @entity { id: ID! type: String }").unwrap();
    assert!(run(&schema, &out).status.success());
    let first = std::fs::read(out.join("migrations/0001_schema.sql")).unwrap();
    std::fs::write(
        &schema,
        "type Account @entity { id: ID! type: String name: String @index }",
    )
    .unwrap();
    let result = run(&schema, &out);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(out.join("migrations/0001_schema.sql")).unwrap(),
        first
    );
    let second = std::fs::read_to_string(out.join("migrations/0002_schema.sql")).unwrap();
    assert!(second.contains("ADD COLUMN \"name\" TEXT"));
    assert!(run(&schema, &out).status.success());
    assert!(!out.join("migrations/0003_schema.sql").exists());
    let generated = std::fs::read_to_string(out.join("entities.rs")).unwrap();
    assert!(generated.contains("pub r#type: Option<String>"));
    std::fs::write(&schema, "type Account @entity { id: ID! }").unwrap();
    assert!(!run(&schema, &out).status.success());
    assert_eq!(
        std::fs::read(out.join("migrations/0001_schema.sql")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read_to_string(out.join("migrations/0002_schema.sql")).unwrap(),
        second
    );
    std::fs::remove_dir_all(root).unwrap();
}
