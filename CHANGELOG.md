# Changelog

## 0.4.0

- Restore registered PostgreSQL application tables on reorgs using a durable
  undo journal for inserts, updates, and deletes.
- Restore entity state and chain progress atomically, including forks inside
  committed batches; retain registrations and undo history across restarts.
- Fetch replacement blocks after live reorgs instead of relying on subscription
  replay.
- Add `PgStore::track_table` and a generated `register_entities` helper; enable
  tracking in examples and the starter. Track tables before indexing. Existing
  data has no retroactive undo history.
- Prune undo history with block retention and reject rewinds beyond available
  history. Tracked handlers receive singleton batches within one transaction.
- Pin Rust 1.96.0 in `rust-toolchain.toml`.
- Ship Linux CLI and example indexer archives with SHA-256 checksums. Crates.io
  publishing requires explicit repository configuration.

See the reorg guide for supported tables, upgrade steps, and limitations.
