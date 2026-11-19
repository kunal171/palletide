# palletide-store

PostgreSQL storage for the Palletide indexing framework.

## Install

```bash
cargo add palletide-store@0.5.1
```

## Usage

`PgStore` keeps the chain cursor, block history, and handler writes in one
transaction. It also runs handler migrations and tracks registered application
tables with a durable undo journal.

Register every related application table with `PgStore::track_table` after its
migrations and before indexing. Reorg rollback restores tracked entity changes
and chain progress atomically. Partition roots and cascading foreign keys are
supported; retained history and schema checkpoints bound how far rollback can go.

See the [rollback guide](https://kunal171.github.io/palletide/reorgs.html) for
registration, permissions, retention, and schema-change requirements.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-store) · [Repository](https://github.com/kunal171/palletide)
