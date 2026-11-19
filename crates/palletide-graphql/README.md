# palletide-graphql

GraphQL serving and query helpers for Palletide indexers, built on async-graphql and Axum.

## Install

```bash
cargo add palletide-graphql@0.5.1
```

## Usage

Serve an application schema with `serve` or embed the HTTP router in an Axum
application. `StatusQuery` reports indexer progress from the framework's stored
block history.

Generated APIs from `palletide-codegen` use this crate for parameterized filters,
ordering, keyset cursors, and entity queries. You can also write resolvers directly
against your PostgreSQL pool.

See the [schema guide](https://kunal171.github.io/palletide/guide.html) for
connecting generated entities and resolvers to your indexer.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-graphql) · [Repository](https://github.com/kunal171/palletide)
