//! The `palletide-codegen` CLI.
//!
//! ```bash
//! palletide-codegen check schema.graphql            # parse + validate, print a summary
//! palletide-codegen generate schema.graphql         # → entities.rs + migration + graphql.rs
//! palletide-codegen new my-indexer                  # scaffold a runnable starter project
//! ```
//!
//! `check` validates a schema (and shows what would be generated); `generate`
//! emits the entity structs + typed upserts, the SQL migration, and the
//! async-graphql read API; `new` scaffolds a whole starter project. See
//! `docs/rfcs/034-schema-first-codegen.md` for the design.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("palletide-codegen {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("check") => match args.get(2) {
            Some(path) => check(Path::new(path)),
            None => {
                eprintln!("error: `check` needs a path to a schema file or directory");
                usage();
                ExitCode::FAILURE
            }
        },
        Some("generate") => {
            // palletide-codegen generate <schema> [--out <dir>]
            let Some(schema) = args.get(2) else {
                eprintln!("error: `generate` needs a path to a schema file or directory");
                usage();
                return ExitCode::FAILURE;
            };
            let out = out_dir(&args).unwrap_or_else(|| "generated".to_string());
            generate(Path::new(schema), Path::new(&out))
        }
        Some("new") => {
            // palletide-codegen new <name> [--out <dir>]
            let Some(name) = args.get(2) else {
                eprintln!("error: `new` needs a project name");
                usage();
                return ExitCode::FAILURE;
            };
            // Default the output directory to ./<name>.
            let out = out_dir(&args).unwrap_or_else(|| name.clone());
            new_project(name, Path::new(&out))
        }
        Some("--help") | Some("-h") | Some("help") | None => {
            usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command `{other}`");
            usage();
            ExitCode::FAILURE
        }
    }
}

/// Parse `--out <dir>` from the args, if present.
fn out_dir(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--out" || a == "-o")?;
    args.get(i + 1).cloned()
}

fn usage() {
    eprintln!(
        "palletide-codegen — schema-first codegen for palletide

USAGE:
    palletide-codegen check <schema.graphql | schema-dir>
        Parse and validate a schema, printing the tables, columns and indexes
        that will be generated. Does not write anything.

    palletide-codegen generate <schema.graphql | schema-dir> [--out <dir>] [--baseline <applied-schema>]
        Generate `entities.rs` (Rust structs + typed upserts), a
        `migrations/NNNN_schema.sql` migration, and `graphql.rs` (async-graphql
        read models + resolvers) into <dir> (default: ./generated). Files carry a
        DO-NOT-EDIT header — edit the schema and re-run, don't hand-edit output.

    palletide-codegen new <name> [--out <dir>]
        Scaffold a runnable starter indexer project into <dir> (default: ./<name>):
        a wired main.rs, a stub handler, schema.graphql, .env.example, and a
        docker-compose.yml for Postgres. `cd <name>`, `cp .env.example .env`, `docker compose up -d`, then
        `WS_URL=… cargo run`.

See docs/rfcs/034-schema-first-codegen.md for the full pipeline."
    );
}

/// Scaffold a new project named `name` into `out`.
fn new_project(name: &str, out: &Path) -> ExitCode {
    if let Err(e) = palletide_codegen::scaffold::validate_name(name) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    // Refuse to write into a non-empty directory, to avoid clobbering.
    if out.exists()
        && out
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        eprintln!("error: `{}` already exists and is not empty", out.display());
        return ExitCode::FAILURE;
    }

    for f in palletide_codegen::scaffold::scaffold(name) {
        let path = out.join(&f.path);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("error: creating `{}`: {e}", parent.display());
                return ExitCode::FAILURE;
            }
        }
        if let Err(e) = std::fs::write(&path, &f.contents) {
            eprintln!("error: writing `{}`: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }

    println!(
        "✓ scaffolded `{name}` into {}\n\n  cd {}\n  cp .env.example .env\n  docker compose up -d\n  WS_URL=wss://rpc.polkadot.io cargo run",
        out.display(),
        out.display(),
    );
    ExitCode::SUCCESS
}

/// Load + parse a schema from a file or directory, printing an error and
/// returning `None` on failure.
fn load(path: &Path) -> Option<palletide_codegen::Schema> {
    let result = if path.is_dir() {
        palletide_codegen::parse_schema_dir(path).map_err(|e| e.to_string())
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| format!("reading `{}`: {e}", path.display()))
            .and_then(|t| palletide_codegen::parse_schema(&t).map_err(|e| e.to_string()))
    };
    match result {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("error: {e}");
            None
        }
    }
}

/// Generate `entities.rs` + a migration into `out`.
fn generate(schema_path: &Path, out: &Path) -> ExitCode {
    let Some(schema) = load(schema_path) else {
        return ExitCode::FAILURE;
    };
    let g = palletide_codegen::generate(&schema);
    let snapshot = out.join(".palletide-schema.json");
    let migrations = out.join("migrations");
    let args: Vec<String> = std::env::args().collect();
    let previous: Option<palletide_codegen::Schema> = if let Some(i) =
        args.iter().position(|s| s == "--baseline")
    {
        let Some(path) = args.get(i + 1) else {
            eprintln!("error: --baseline requires the previously applied schema");
            return ExitCode::FAILURE;
        };
        let Some(old) = load(Path::new(path)) else {
            return ExitCode::FAILURE;
        };
        Some(old)
    } else if snapshot.exists() {
        match std::fs::read_to_string(&snapshot)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(value) => Some(value),
            None => {
                eprintln!("error: invalid schema snapshot; restore it or provide --baseline");
                return ExitCode::FAILURE;
            }
        }
    } else if migrations.join("0001_schema.sql").exists() {
        if std::fs::read_to_string(migrations.join("0001_schema.sql"))
            .ok()
            .as_deref()
            == Some(g.migration_sql.as_str())
        {
            Some(schema.clone())
        } else {
            eprintln!("error: existing migrations have no snapshot; pass --baseline <previous-schema.graphql>");
            return ExitCode::FAILURE;
        }
    } else {
        None
    };
    let planned = match previous
        .as_ref()
        .map(|old| palletide_codegen::migration::plan(old, &schema))
        .transpose()
    {
        Ok(Some(sql)) => sql,
        Ok(None) => Some(g.migration_sql.clone()),
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&migrations) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    let entities_path = out.join("entities.rs");
    let graphql_path = out.join("graphql.rs");
    let mut migration_path = migrations.join("(unchanged)");
    if let Some(sql) = planned {
        let entries = match std::fs::read_dir(&migrations) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("error reading migrations: {error}");
                return ExitCode::FAILURE;
            }
        };
        let entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        let existing = entries
            .iter()
            .find(|p| std::fs::read_to_string(p).ok().as_deref() == Some(sql.as_str()));
        if let Some(path) = existing {
            migration_path = path.clone();
        } else {
            let version = entries
                .iter()
                .filter_map(|p| {
                    p.file_name()?
                        .to_str()?
                        .split('_')
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .max()
                .unwrap_or(0)
                + 1;
            migration_path = migrations.join(format!("{version:04}_schema.sql"));
            use std::io::Write;
            let written = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&migration_path)
                .and_then(|mut f| f.write_all(sql.as_bytes()));
            if let Err(error) = written {
                eprintln!("error writing migration: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    for (path, contents) in [
        (&entities_path, &g.entities_rs),
        (&graphql_path, &g.graphql_rs),
    ] {
        if let Err(error) = std::fs::write(path, contents) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    }
    let temporary = out.join(".palletide-schema.json.tmp");
    if let Err(error) = std::fs::write(&temporary, serde_json::to_vec_pretty(&schema).unwrap())
        .and_then(|_| std::fs::rename(&temporary, &snapshot))
    {
        eprintln!("error writing schema snapshot: {error}");
        return ExitCode::FAILURE;
    }

    println!(
        "✓ generated {} entit{} into {}:\n    {}\n    {}\n    {}",
        schema.entities.len(),
        if schema.entities.len() == 1 {
            "y"
        } else {
            "ies"
        },
        out.display(),
        entities_path.display(),
        migration_path.display(),
        graphql_path.display(),
    );
    ExitCode::SUCCESS
}

/// Parse a schema (file or directory) and print what it resolves to.
fn check(path: &Path) -> ExitCode {
    let Some(schema) = load(path) else {
        return ExitCode::FAILURE;
    };

    println!(
        "✓ {} — {} entit{}, {} enum{}",
        path.display(),
        schema.entities.len(),
        if schema.entities.len() == 1 {
            "y"
        } else {
            "ies"
        },
        schema.enums.len(),
        if schema.enums.len() == 1 { "" } else { "s" },
    );
    for e in &schema.entities {
        println!("\n  {} → table `{}`", e.name, e.table);
        for f in &e.fields {
            let mut notes = Vec::new();
            if f.is_id {
                notes.push("PRIMARY KEY");
            }
            if f.unique {
                notes.push("UNIQUE");
            }
            if f.indexed {
                notes.push("INDEX");
            }
            if !f.nullable && !f.is_id {
                notes.push("NOT NULL");
            }
            let suffix = if notes.is_empty() {
                String::new()
            } else {
                format!("  [{}]", notes.join(", "))
            };
            println!(
                "    {:<24} {:<20} {}{}",
                f.column,
                f.pg_type(),
                f.rust_type(),
                suffix
            );
        }
    }
    if !schema.enums.is_empty() {
        println!("\n  enums (stored as TEXT):");
        for en in &schema.enums {
            println!("    {} = {}", en.name, en.values.join(" | "));
        }
    }
    ExitCode::SUCCESS
}
