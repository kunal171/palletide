# palletide-codegen

Schema tools and project scaffolding for Palletide, a Rust framework for indexing Substrate chains.

## Install

```bash
cargo install palletide-codegen --version 0.5.1 --locked
```

## Use

Create an indexer project:

```bash
palletide-codegen new my-indexer
```

Validate a GraphQL entity schema and generate Rust code:

```bash
palletide-codegen check schema.graphql
palletide-codegen generate schema.graphql --out src/generated
```

The generator produces entities, typed and bulk upserts, numbered SQL migrations,
and GraphQL resolvers. Commit the generated schema snapshot alongside migrations.
Generation does not apply migrations or wire handlers into your application.

See the [CLI reference](https://kunal171.github.io/palletide/reference/codegen-cli.html)
for commands, schema baselines, and incremental migrations.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-codegen) · [Repository](https://github.com/kunal171/palletide)
