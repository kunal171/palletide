# Palletide

Rust-native indexing for Substrate chains.

Fetch blocks, map events in Rust, and store the results in PostgreSQL. Palletide
handles historical backfill, live finalized blocks, and resumable progress. An
optional schema tool generates entities, migrations, upserts, and a GraphQL API.

[Documentation](https://kunal171.github.io/palletide/) ·
[Build an indexer](docs/GUIDE.md) ·
[Examples](examples/)

## Get started

You need Rust 1.96+ and Docker for PostgreSQL.

Install the CLI from GitHub (the renamed crates are not yet published):

```bash
cargo install --git https://github.com/kunal171/palletide --locked palletide-codegen
```

Create and run an indexer:

```bash
palletide-codegen new my-indexer
cd my-indexer
cp .env.example .env
# Set WS_URL in .env to your chain's WebSocket RPC endpoint.
docker compose up -d
cargo run
```

The generated project includes a sample handler and database configuration.
Replace the handler's event mapping with your chain's events.

## Generate entities and an API

Define your tables in `schema.graphql`:

```graphql
type Transfer @entity {
  id: ID!
  blockHeight: Int! @index
  from: String!
  amount: BigInt!
}
```

Then run the installed CLI:

```bash
palletide-codegen generate schema.graphql --out src/generated
```

This generates Rust entities with typed upserts, a SQL migration, and GraphQL
resolvers. Follow the [schema guide](https://kunal171.github.io/palletide/guide.html#2-define-your-schema)
to wire them into your handler and server. Generation does not apply migrations
or modify your application wiring.

## How it works

```text
RPC / SQD portal → decoded blocks → your handlers → PostgreSQL → GraphQL
```

- Dynamic decoding uses metadata for each block's runtime version.
- Handler writes and progress commit together in one transaction per batch.
- RPC, SQD portal backfill, and hybrid sources share the same handler interface.
- Reorg detection locates the fork and rewinds framework progress. Entity rollback
  is not implemented yet; see [reorg limitations](https://kunal171.github.io/palletide/reorgs.html).

See the [transfers](examples/transfers/) and [multi-pallet](examples/multi-pallet/)
examples for complete applications, or the [starter template](templates/starter/)
for a schema-based indexer with GraphQL already wired.

## Development

```bash
cargo install --path crates/palletide-codegen --locked
cargo test --workspace --all-features
```

Database and live-chain integration tests are opt-in. See
[CONTRIBUTING.md](CONTRIBUTING.md) for setup and checks.

Alpha software; APIs may change. Licensed under [Apache-2.0](LICENSE).
