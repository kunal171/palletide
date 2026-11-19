# `palletide-codegen` CLI

## Install the CLI

Requires Rust 1.96 or newer. Install from crates.io:

```bash
cargo install palletide-codegen --version 0.5.1 --locked
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

## Evolve a schema safely

`generate` records `.palletide-schema.json` beside generated Rust. Commit this
snapshot and every SQL migration. Re-running an unchanged schema adds no SQL;
adding nullable fields, tables or indexes creates the next numbered migration.
Existing migration files are never rewritten.

Destructive changes, type changes, new required columns, and uniqueness changes
need an explicit data migration. Write and review that migration first, then
use `--baseline previous-schema.graphql` when bootstrapping an existing output
without a snapshot. To adopt an already-applied manual migration, pass the
applied schema to `--baseline`; this explicitly replaces the stored baseline.
Retain every applied SQL migration file.
The CLI does not apply migrations to a database.

Each incremental DDL migration checkpoints entity undo history. Reorgs before
the schema boundary are rejected; see the [reorg guide](../reorgs.md).

## Generated queries

Generated lists accept `limit`, `id`, `filters`, `orderBy`, `descending`, and
`after`. Filters support one of `eq`, `gt`, `lt`, or `isNull`, on indexed, unique,
ID or relation fields. Values are strings cast to the schema's PostgreSQL type;
array filter values are JSON arrays encoded as strings.

```graphql
{
  transfers(limit: 25, orderBy: "amount", descending: false,
    filters: [{field: "amount", gt: "100000000000000000000"}]) {
    id
    amount
    owner { id }
    _cursor(orderBy: "amount", descending: false)
  }
}
```

Pass the last row's `_cursor` into `after` for the next page, with the same
ordering options. Cursors include the ordering value and ID tiebreaker; NULLs
sort last. A cursor with different ordering is rejected. Forward entity fields
resolve to objects, and `@derivedFrom` fields resolve reverse relations.
List relations are bounded; a singular reverse relation errors if it is not
unique. This changes relation fields from the earlier ID-string query API.

Entities also expose `upsert_many(rows, connection)` for parameterized bulk
writes. Use it for rows belonging to one block, on the processor transaction.
Duplicate IDs in one bulk input are rejected before any statement is executed.
See the [compiled example](https://github.com/kunal171/palletide/tree/main/examples/schema-api).

When upgrading an existing application, add `serde = { version = "1", features =
["derive"] }` and `serde_json = "1"` to its dependencies before regenerating
query models. Newly scaffolded applications already include them.
