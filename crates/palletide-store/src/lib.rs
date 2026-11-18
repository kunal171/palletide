//! # palletide-store
//!
//! A Postgres-backed [`Store`](palletide_core::Store) for the palletide framework.
//!
//! The store owns the framework's *own* bookkeeping — the `(height, hash)`
//! cursor and the reorg rollback — and hands handlers a `sqlx` transaction so
//! their entity writes commit atomically with the cursor advance.
//!
//! Register application tables with [`PgStore::track_table`] after their
//! migrations. Row triggers retain before-images so a reorg restores inserts,
//! updates and deletes atomically with the framework cursor.

mod config;
mod schema;
mod store;

pub use config::StoreConfig;
pub use schema::MIGRATOR;
pub use store::{handler_migrations_table, PgStore};

/// Re-export of sqlx's `Migrator` so handlers can run their own embedded
/// migrations (via `sqlx::migrate!`) through [`PgStore::run_handler_migrations`]
/// without depending on `sqlx` directly.
pub use sqlx::migrate::Migrator;
