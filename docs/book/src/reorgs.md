# Reorgs & finality

Before committing a batch, the processor compares its first block's parent hash
with the stored hash. On a mismatch it walks back to the common ancestor,
restores registered application tables and rewinds progress in one database
transaction, then fetches the replacement blocks. Live following also fetches
the missing range; it does not depend on the subscription replaying history.

## Register application tables

After creating tables or running migrations in `Handler::init`, register every
table containing chain-derived state:

```rust,ignore
store.track_table("accounts").await?;
store.track_table("transfers").await?;
```

For generated entities, call the generated helper after applying migrations:

```rust,ignore
store.run_handler_migrations(&MIGRATOR, self.name()).await?;
crate::generated::entities::register_entities(store).await?;
```

Registration is idempotent and survives restarts. The examples and starter
register their application tables. Unregistered tables are not restored.

## What rollback restores

PostgreSQL row triggers record before-images in `palletide_entity_undo`, on the
same transaction as handler writes. On a fork, changes above the ancestor are
undone in reverse order:

- Inserts are removed.
- Updates restore the previous row, including a changed primary key.
- Deletes reinsert the previous row.

This restores mutable entities as well as event logs. Undo records, entity data,
and the progress cursor commit or roll back together. A failed handler leaves
no entity changes or journal entries. A failed restoration leaves the cursor,
entities, and journal unchanged.

## Exact block attribution

When any tables are tracked, `prepare` and `process_batch` receive one block at a
time. Handler preparation still runs concurrently across handlers; writes run
serially in block order. A fetched batch still commits in one transaction.
This prevents a fork inside a previously committed batch from restoring the
wrong entity state. It reduces cross-block bulk-write opportunities and adds
journal storage and trigger overhead. Untracked stores keep the bulk batch path.

Manual writes to tracked tables must use a store transaction and set context:

```rust,ignore
let mut tx = store.begin().await?;
store.set_block_context(&mut tx, block.id.number).await?;
// Write application rows, then call set_cursor and commit on this transaction.
```

Writes without block context fail instead of bypassing the undo journal. Keep
operational data that is independent of the chain in untracked tables.

## Retention and upgrades

`REORG_RETENTION` prunes old block records and undo entries together. Keep it at
least `MAX_REORG_DEPTH`. A rollback beyond available undo history fails without
changing data; `REORG_RETENTION=0` retains all history.

The undo tables are added by a new framework migration. Existing Palletide
migration checksums are preserved. On an existing database, tracking begins at
the current cursor: earlier writes have no before-images. A fork below that
baseline fails and requires a clean reindex. Existing Subdex installations must
also follow the [rename migration guidance](./renaming.md).

## Supported tables and limits

Tracked tables must be ordinary PostgreSQL tables with a primary key. Composite
keys, quoted names, generated columns, identity columns, JSON, and numeric values
are supported. Standard foreign keys may be used; deferrable constraints are
deferred during restoration.

Persistent partition roots and cascading foreign keys are supported. Register the
partition root rather than individual partitions. During restoration, immediate
foreign-key checks are temporarily deferred, checked against the completed
state, and restored to their original definitions in the same transaction.
The tracking user must own the affected tables. Ordinary inheritance and
unlogged/temporary tables are rejected. TRUNCATE is rejected;
use DELETE so row changes can be journaled. Sequences are not rewound. Register
all related chain-derived tables before indexing, use a schema checkpoint for DDL changes, and ensure custom triggers do not introduce
untracked side effects. Disabling triggers or modifying tracking metadata is
outside the rollback contract.

The built-in sources follow finalized blocks, where normal chain forks are not
expected. The parent-hash check protects against divergence within the retained
window. `MAX_REORG_DEPTH` bounds the ancestor walk; a deeper fork stops indexing.

## Schema checkpoints

Apply trusted application DDL with `store.apply_schema_change(sql).await?`.
The operation clears older undo records, establishes a new rollback baseline,
applies the DDL and refreshes tracking in one transaction. A failing migration
rolls all of that back. New blocks remain reversible, but a reorg before this
schema boundary requires a reindex. Generated incremental migrations include
`palletide_checkpoint_entities()` before their DDL; run them with SQLx's default
transactional migrations, never `no-transaction` mode.

The processor rejects gaps, disconnected batches, unverifiable old replays and
unknown fork ancestors. Matching committed blocks are skipped. Reconnect gaps
are fetched before a live tip is applied. Finite retention never serves as proof
that a missing block hash is a common ancestor.

Framework bookkeeping lives in PostgreSQL's `public` schema; application tables
may use other schemas when registered with qualified names.
