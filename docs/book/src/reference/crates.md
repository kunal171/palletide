# Crates

palletide is a Cargo workspace of focused crates. Depend on the ones you need; the
`palletide` façade re-exports the engine and core types.

| Crate | Role |
|---|---|
| [`palletide-core`](https://github.com/kunal171/palletide/tree/main/crates/palletide-core) | Core traits (`DataSource`, `Handler`, `Store`, `ProcessorObserver`) and chain-agnostic types (`Block`, `Event`, …). No async runtime, no DB — anything can implement the contracts. |
| [`palletide`](https://github.com/kunal171/palletide/tree/main/crates/palletide) | The indexing engine: drives a `DataSource` through `Handler`s into a `Store`, with resumable progress and reorg handling. Optional `metrics` feature (Prometheus). |
| [`palletide-source`](https://github.com/kunal171/palletide/tree/main/crates/palletide-source) | `SubxtSource` (direct RPC), `SqdPortalSource` + `HybridSource` (`sqd` feature), and the `value` field-extraction helpers for handlers. |
| [`palletide-store`](https://github.com/kunal171/palletide/tree/main/crates/palletide-store) | `PgStore`: the Postgres-backed `Store` — cursor, reorg rollback, handler migrations, deferred indexes. |
| [`palletide-graphql`](https://github.com/kunal171/palletide/tree/main/crates/palletide-graphql) | GraphQL serving (async-graphql + axum), including the built-in `indexerStatus` query. |
| [`palletide-config`](https://github.com/kunal171/palletide/tree/main/crates/palletide-config) | Typed, layered config: `palletide.toml` overlaid by environment variables. |
| [`palletide-codegen`](https://github.com/kunal171/palletide/tree/main/crates/palletide-codegen) | The `palletide-codegen` CLI: `schema.graphql` → entity structs + migration + typed upserts + GraphQL API, plus the project scaffolder. |

See [Architecture](../architecture.md) for how they fit together.
