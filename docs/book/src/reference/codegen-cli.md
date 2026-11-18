# `palletide-codegen` CLI

## Install the CLI

Requires Rust 1.96 or newer. Install from GitHub until the renamed crates are
published on crates.io:

```bash
cargo install --git https://github.com/kunal171/palletide --locked palletide-codegen
palletide-codegen --help
```

From a local checkout, use `cargo install --path crates/palletide-codegen --locked`.
You can also run `cargo run -p palletide-codegen -- <command>` without installing.
If the installed command is not found, add Cargo's binary directory (usually
`~/.cargo/bin`) to your `PATH`.

## `check`

```bash
palletide-codegen check <schema.graphql | schema-dir>
```

Parse and validate a schema, printing the tables, columns, and indexes that would
be generated. Writes nothing — useful for catching a bad schema before it becomes
a bad table.

## `generate`

```bash
palletide-codegen generate <schema.graphql | schema-dir> [--out <dir>]
```

Generate, into `<dir>` (default `./generated`):

- `entities.rs` — one struct per `@entity`, **plus a typed `.upsert()`**;
- `migrations/0001_schema.sql` — `CREATE TABLE` + indexes;
- `graphql.rs` — an async-graphql read API (a list + count resolver per entity,
  merged with the framework's `indexerStatus`).

All files carry a `DO NOT EDIT` header. Edit the schema and re-run — never
hand-edit the output. Pass a **directory** to concatenate a modular
`schema/*.graphql`.

## `new`

```bash
palletide-codegen new <name> [--out <dir>]
```

Scaffold a runnable starter project into `<dir>` (default `./<name>`): a wired
`main.rs`, a stub handler, `schema.graphql`, `.env.example`, and a
`docker-compose.yml`. Then:

```bash
cd <name>
cp .env.example .env
docker compose up -d
WS_URL=wss://rpc.polkadot.io cargo run
```

See the [starter template](../starter-template.md) for the schema-first variant
that also commits the generated code.

## Enable entity rollback

After applying the generated migrations in `Handler::init`, call
`generated::entities::register_entities(store).await?`. This registers every
entity table for durable reorg undo. Regenerating Rust entities does not modify
already-applied SQL migration files. See [reorgs](../reorgs.md).
