# palletide

The indexing engine for Palletide, a Rust framework for indexing Substrate chains into PostgreSQL.

## Install

```bash
cargo add palletide@0.5.1
```

## Usage

`Processor` resumes from a stored cursor, backfills historical blocks, follows
finalized blocks, and checks chain continuity. Handlers write application data
inside the store transaction. Enable the `metrics` feature for Prometheus metrics.

Pair it with [`palletide-source`](https://crates.io/crates/palletide-source) and
[`palletide-store`](https://crates.io/crates/palletide-store), then implement the
`Handler` trait for your application.

Application rollback requires registering tables with `PgStore::track_table` or
the generated `register_entities` helper before indexing.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide) · [Repository](https://github.com/kunal171/palletide)
