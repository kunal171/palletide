# Moving to Palletide

Palletide was previously named Subdex. This rename changes crate names, Rust
imports, CLI commands, configuration, and framework table names.

- Replace `subdex-*` dependencies with `palletide-*`, and `subdex_*` Rust imports
  with `palletide_*`. The root crate is `palletide`.
- Install `palletide-codegen` using the [CLI installation instructions](./reference/codegen-cli.md).
- Rename `subdex.toml` to `palletide.toml` and `SUBDEX_CONFIG` to `PALLETIDE_CONFIG`.
  Standard variables such as `WS_URL` and `DATABASE_URL` keep their names.
- Update repository links to `https://github.com/kunal171/palletide` and docs links
  to `https://kunal171.github.io/palletide/`.

## Existing databases

New examples use a database named `palletide`. Keep your existing `DATABASE_URL`
if you want to retain an existing database; the database name itself need not change.

The initial framework migration now creates `palletide_block` and
`palletide_deferred_index`. Do not run the renamed framework against an existing
Subdex database without planning a migration: SQLx tracks migration checksums,
and the initial migration's checksum has changed. Simply renaming the tables
will not update SQLx's migration history.

For disposable development data, start with a new database. For retained data,
back up first and prepare a separate database migration covering the framework
tables and SQLx migration history before deploying. Handler tables retain their
application-defined names.
