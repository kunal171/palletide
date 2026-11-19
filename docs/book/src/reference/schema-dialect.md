# The `@entity` schema dialect

`palletide-codegen` reads a Subsquid-compatible `schema.graphql`. Each `@entity` type
becomes a table; `palletide-codegen generate` turns the schema into entity structs,
a SQL migration, typed upserts, and a GraphQL API.

## Entities

```graphql
type Transfer @entity {
    id: ID!                    # required primary key
    blockHeight: Int! @index
    from: String!
    to: String                 # nullable
    amount: BigInt!
    tags: [String!]!           # a scalar list → a Postgres array
    meta: JSON
}
```

Every `@entity` needs a non-null `id: ID!`, which becomes `TEXT PRIMARY KEY`.

## Scalars

| GraphQL | Rust | Postgres |
|---|---|---|
| `ID`, `String`, `Bytes` | `String` | `TEXT` |
| `Int` | `i32` | `INTEGER` |
| `BigInt` | `String` (decimal) | `NUMERIC` |
| `Float` | `f64` | `DOUBLE PRECISION` |
| `Boolean` | `bool` | `BOOLEAN` |
| `DateTime` | `i64` (ms) | `BIGINT` |
| `JSON` | `serde_json::Value` | `JSONB` |

`BigInt` is carried as a **decimal string**, not `i64`: chain balances routinely
exceed `i64::MAX`. A **list of scalars** (`[String!]!`) becomes a native Postgres
array (`TEXT[]`) and a `Vec<T>` in Rust.

## Directives

- `@index` — a non-unique index on the column.
- `@unique` — a `UNIQUE` constraint.
- `@derivedFrom` — a virtual reverse relation; it generates no storage column
  and is served by a generated GraphQL resolver.

## Enums & relations

- `enum` types are stored as `TEXT`; the generated Rust enum round-trips to/from
  its variant name.
- A field typed as another `@entity` is a **relation stored as that entity's `id`
  string** in Rust entities and PostgreSQL. The GraphQL read model resolves the
  referenced object. Reverse relations use `@derivedFrom(field: "owner")` on a
  list or singular field; singular results must be unique.

Anything outside this dialect is a **clear error** rather than a silently-wrong
table. The parser is validated against a real 100+ entity production schema. See
[RFC 034](../rfcs/034-schema-first-codegen.md) for the design.

## Generation and updates

SQL identifiers are quoted and Rust keywords are escaped (`type` becomes
`r#type`). Generated framework names and normalized name collisions produce
clear errors. Generated queries require `serde` with its `derive` feature and
`serde_json`; scaffolded projects include both dependencies.

Commit the generated `.palletide-schema.json` snapshot and immutable numbered SQL
migrations. See the [CLI reference](./codegen-cli.md) for schema evolution,
filters, cursor pagination, relations, and bulk writes.
