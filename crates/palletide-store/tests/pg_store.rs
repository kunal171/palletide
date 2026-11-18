//! Integration tests for [`PgStore`] against a real Postgres.
//!
//! **Database-dependent** and therefore `#[ignore]`d by default. Run with a
//! Postgres available:
//!
//! ```bash
//! docker run -d --name pg -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=palletide \
//!     -p 55432:5432 postgres:16-alpine
//! PALLETIDE_TEST_DB=postgres://postgres:postgres@localhost:55432/palletide \
//!     cargo test -p palletide-store --test pg_store -- --ignored --nocapture
//! ```
//!
//! Each test uses a uniquely-named throwaway database created/dropped around it,
//! so they are isolated and can run concurrently.

use palletide_core::{Block, BlockId, Store};
use palletide_store::{PgStore, StoreConfig};
use sqlx::{Connection, PgConnection};

fn admin_url() -> String {
    std::env::var("PALLETIDE_TEST_DB")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:55432/palletide".to_string())
}

/// Replace the database name in a postgres URL.
fn with_db(url: &str, db: &str) -> String {
    let (base, _old) = url.rsplit_once('/').expect("url has a /db part");
    format!("{base}/{db}")
}

/// Create a fresh throwaway database and return its URL. The caller drops it via
/// [`drop_db`].
async fn make_db(suffix: &str) -> String {
    let admin = admin_url();
    let db = format!("palletide_test_{suffix}");
    let mut conn = PgConnection::connect(&admin).await.expect("connect admin");
    // Defensive: drop if a previous run leaked it. `raw_sql` takes an owned
    // String (sqlx 0.9 requires the query to be 'static for DDL).
    let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {db} WITH (FORCE)"
    )))
    .execute(&mut conn)
    .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db}")))
        .execute(&mut conn)
        .await
        .expect("create db");
    conn.close().await.ok();
    with_db(&admin, &db)
}

async fn drop_db(suffix: &str) {
    let admin = admin_url();
    let db = format!("palletide_test_{suffix}");
    if let Ok(mut conn) = PgConnection::connect(&admin).await {
        let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {db} WITH (FORCE)"
        )))
        .execute(&mut conn)
        .await;
        conn.close().await.ok();
    }
}

fn block(number: u32, hash: &str, parent: &str, spec: u32) -> Block {
    Block {
        id: BlockId {
            number,
            hash: hash.into(),
        },
        parent_hash: parent.into(),
        timestamp: Some(1_700_000_000_000 + number as u64),
        spec_version: spec,
        finalized: true,
        extrinsics: vec![],
        events: vec![],
    }
}

/// Commit one block through the store's transaction lifecycle.
async fn commit_block(store: &PgStore, b: &Block) {
    let mut tx = store.begin().await.expect("begin");
    store.set_cursor(&mut tx, b).await.expect("set_cursor");
    store.commit(tx).await.expect("commit");
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn init_is_idempotent_and_cursor_starts_empty() {
    let url = make_db("init").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");

    store.init().await.expect("init");
    // Running init again must not error (migrations are idempotent).
    store.init().await.expect("init again");

    assert!(
        store.cursor().await.expect("cursor").is_none(),
        "no blocks yet"
    );
    assert!(store.hash_at(1).await.expect("hash_at").is_none());

    drop_db("init").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn commit_advances_cursor_and_records_metadata() {
    let url = make_db("commit").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("init");

    commit_block(&store, &block(1, "0xaa", "0x00", 145)).await;
    commit_block(&store, &block(2, "0xbb", "0xaa", 145)).await;
    commit_block(&store, &block(3, "0xcc", "0xbb", 147)).await;

    let cursor = store.cursor().await.expect("cursor").expect("some");
    assert_eq!(cursor.number, 3, "cursor is the highest block");
    assert_eq!(cursor.hash, "0xcc");

    assert_eq!(store.hash_at(2).await.expect("h2"), Some("0xbb".into()));
    assert_eq!(store.hash_at(99).await.expect("h99"), None);

    drop_db("commit").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn set_cursor_upsert_is_idempotent() {
    let url = make_db("upsert").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("init");

    commit_block(&store, &block(5, "0xold", "0x04", 145)).await;
    // Re-commit the same height with a different hash (e.g. after a rollback the
    // corrected chain re-indexes height 5). ON CONFLICT should update in place.
    commit_block(&store, &block(5, "0xnew", "0x04", 146)).await;

    let cursor = store.cursor().await.expect("cursor").expect("some");
    assert_eq!(cursor.number, 5);
    assert_eq!(cursor.hash, "0xnew", "re-commit updated the hash in place");

    drop_db("upsert").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn rollback_removes_blocks_above_fork() {
    let url = make_db("rollback").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("init");

    for n in 1..=5 {
        let parent = if n == 1 {
            "0x00".to_string()
        } else {
            format!("0x{:02x}", n - 1)
        };
        commit_block(&store, &block(n, &format!("0x{n:02x}"), &parent, 145)).await;
    }
    assert_eq!(store.cursor().await.expect("c").expect("s").number, 5);

    // Reorg detected at height 3: roll back everything above 3.
    store.rollback_to(3).await.expect("rollback");

    let cursor = store.cursor().await.expect("c").expect("s");
    assert_eq!(cursor.number, 3, "cursor back at fork point");
    assert!(
        store.hash_at(4).await.expect("h4").is_none(),
        "height 4 removed"
    );
    assert!(
        store.hash_at(5).await.expect("h5").is_none(),
        "height 5 removed"
    );
    assert!(
        store.hash_at(3).await.expect("h3").is_some(),
        "height 3 retained"
    );

    drop_db("rollback").await;
}

/// Count rows in the bookkeeping table.
async fn block_row_count(store: &PgStore) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM palletide_block")
        .fetch_one(store.pool())
        .await
        .expect("count")
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn pruning_keeps_palletide_block_bounded() {
    // Acceptance: with a retention window of 10, indexing 50 blocks must leave
    // the table bounded (~the window), not 50 rows.
    let url = make_db("prune_bounded").await;
    let store = PgStore::connect(StoreConfig::new(&url).with_reorg_retention(10))
        .await
        .expect("connect");
    store.init().await.expect("init");

    for n in 1..=50 {
        let parent = if n == 1 {
            "0x00".to_string()
        } else {
            format!("0x{:02x}", n - 1)
        };
        commit_block(&store, &block(n, &format!("0x{n:02x}"), &parent, 145)).await;
    }

    // Cursor is still authoritative at the head.
    assert_eq!(store.cursor().await.expect("c").expect("s").number, 50);

    // Rows below (50 - 10) = 40 are pruned; heights 40..=50 (11 rows) remain.
    let count = block_row_count(&store).await;
    assert_eq!(
        count, 11,
        "bounded to the retention window, got {count} rows"
    );
    assert!(
        store.hash_at(39).await.expect("h39").is_none(),
        "pruned below the window"
    );
    assert!(
        store.hash_at(40).await.expect("h40").is_some(),
        "window boundary retained"
    );
    assert!(
        store.hash_at(50).await.expect("h50").is_some(),
        "head retained"
    );

    drop_db("prune_bounded").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn reorg_within_the_window_still_works_after_pruning() {
    // Acceptance: pruning must not break reorg detection/rollback INSIDE the
    // retained window.
    let url = make_db("prune_reorg").await;
    let store = PgStore::connect(StoreConfig::new(&url).with_reorg_retention(10))
        .await
        .expect("connect");
    store.init().await.expect("init");

    for n in 1..=50 {
        let parent = if n == 1 {
            "0x00".to_string()
        } else {
            format!("0x{:02x}", n - 1)
        };
        commit_block(&store, &block(n, &format!("0x{n:02x}"), &parent, 145)).await;
    }

    // A reorg at height 45 (well inside the 10-block window): the hash is still
    // there to compare against, and rollback drops everything above it.
    assert!(
        store.hash_at(45).await.expect("h45").is_some(),
        "fork point still in the retained window"
    );
    store.rollback_to(45).await.expect("rollback");

    assert_eq!(
        store.cursor().await.expect("c").expect("s").number,
        45,
        "cursor rolled back to the fork"
    );
    assert!(store.hash_at(46).await.expect("h46").is_none());
    assert!(store.hash_at(50).await.expect("h50").is_none());

    drop_db("prune_reorg").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn retention_zero_disables_pruning() {
    // Acceptance: pruning is configurable and can be disabled (full audit trail).
    let url = make_db("prune_off").await;
    let store = PgStore::connect(StoreConfig::new(&url).with_reorg_retention(0))
        .await
        .expect("connect");
    store.init().await.expect("init");

    for n in 1..=30 {
        let parent = if n == 1 {
            "0x00".to_string()
        } else {
            format!("0x{:02x}", n - 1)
        };
        commit_block(&store, &block(n, &format!("0x{n:02x}"), &parent, 145)).await;
    }

    assert_eq!(
        block_row_count(&store).await,
        30,
        "retention 0 keeps every row"
    );
    assert!(
        store.hash_at(1).await.expect("h1").is_some(),
        "genesis row retained when pruning is off"
    );

    drop_db("prune_off").await;
}

// --- Handler-owned migrations (#33) ---

/// A handler's own embedded migrations (two versions: create, then evolve).
static HANDLER_MIGRATOR: palletide_store::Migrator =
    sqlx::migrate!("./tests/fixtures/handler_migrations");

/// Does `table` exist in the current database?
async fn table_exists(store: &PgStore, table: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
        .bind(table)
        .fetch_one(store.pool())
        .await
        .expect("to_regclass")
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn handler_migrations_apply_once_in_order_and_are_idempotent() {
    let url = make_db("handler_migrations").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("framework init");

    // Apply the handler's migrations.
    store
        .run_handler_migrations(&HANDLER_MIGRATOR, "widgets")
        .await
        .expect("run handler migrations");

    // Both versions applied, in order: the table exists AND has the v2 column.
    assert!(
        table_exists(&store, "widgets").await,
        "0001 created the table"
    );
    let has_colour: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'widgets' AND column_name = 'colour')",
    )
    .fetch_one(store.pool())
    .await
    .expect("column check");
    assert!(has_colour, "0002 added the colour column");

    // Recorded in the handler's OWN tracking table…
    assert!(
        table_exists(&store, "_sqlx_migrations_widgets").await,
        "handler migrations tracked in their own table"
    );
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations_widgets")
        .fetch_one(store.pool())
        .await
        .expect("count applied");
    assert_eq!(applied, 2, "both handler migrations recorded");

    // …isolated from the framework's own migration table, which only tracks the
    // bookkeeping schema (this is the collision the per-handler table prevents).
    let framework: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(store.pool())
        .await
        .expect("count framework");
    assert_eq!(
        framework,
        palletide_store::MIGRATOR.iter().count() as i64,
        "framework table untouched by handler migrations"
    );

    // Idempotent: re-running applies nothing new and does not error (a fresh DB
    // and an already-migrated DB converge).
    store
        .run_handler_migrations(&HANDLER_MIGRATOR, "widgets")
        .await
        .expect("re-run is idempotent");
    let applied_again: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations_widgets")
        .fetch_one(store.pool())
        .await
        .expect("count applied again");
    assert_eq!(applied_again, 2, "re-run applied nothing new");

    drop_db("handler_migrations").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn two_handlers_get_separate_migration_tables() {
    // Two handlers running migrations must not collide on one tracking table.
    let url = make_db("handler_migrations_two").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("framework init");

    store
        .run_handler_migrations(&HANDLER_MIGRATOR, "alpha")
        .await
        .expect("alpha");
    // The same migration set under a different handler name tracks separately.
    // (Its DDL is IF NOT EXISTS / would conflict, so we only assert the tables.)
    assert!(table_exists(&store, "_sqlx_migrations_alpha").await);
    assert!(
        !table_exists(&store, "_sqlx_migrations_beta").await,
        "beta hasn't run yet"
    );

    drop_db("handler_migrations_two").await;
}

/// Whether an index named `name` exists in the current database.
async fn index_exists(store: &PgStore, name: &str) -> bool {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT indexname::text FROM pg_indexes WHERE indexname = $1")
            .bind(name)
            .fetch_optional(store.pool())
            .await
            .expect("query pg_indexes");
    row.is_some()
}

async fn deferred_count(store: &PgStore) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM palletide_deferred_index")
        .fetch_one(store.pool())
        .await
        .expect("count deferred");
    n
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn defer_index_drops_on_fresh_and_recreates_at_backfill_complete() {
    let url = make_db("defer_idx").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("init");

    // A handler table with an index that already exists (as `init` would create).
    sqlx::query("CREATE TABLE t (id BIGINT PRIMARY KEY, k TEXT)")
        .execute(store.pool())
        .await
        .expect("create table");
    let create_idx = "CREATE INDEX IF NOT EXISTS t_k_idx ON t (k)";
    sqlx::query(create_idx)
        .execute(store.pool())
        .await
        .expect("create index");
    assert!(
        index_exists(&store, "t_k_idx").await,
        "index exists to start"
    );

    // Fresh DB (no cursor) → defer drops the index and registers it.
    store
        .defer_index("t_k_idx", create_idx)
        .await
        .expect("defer_index");
    assert!(
        !index_exists(&store, "t_k_idx").await,
        "deferred index is dropped during backfill"
    );
    assert_eq!(deferred_count(&store).await, 1, "registered as pending");

    // Reaching head → recreate + clear the registry.
    store
        .on_backfill_complete()
        .await
        .expect("on_backfill_complete");
    assert!(
        index_exists(&store, "t_k_idx").await,
        "index recreated at head"
    );
    assert_eq!(deferred_count(&store).await, 0, "registry cleared");

    // Idempotent: calling again with nothing pending is a no-op.
    store
        .on_backfill_complete()
        .await
        .expect("second on_backfill_complete is a no-op");

    drop_db("defer_idx").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn defer_index_on_a_resumed_db_only_ensures_the_index() {
    let url = make_db("defer_idx_resume").await;
    let store = PgStore::connect(StoreConfig::new(&url))
        .await
        .expect("connect");
    store.init().await.expect("init");
    sqlx::query("CREATE TABLE t (id BIGINT PRIMARY KEY, k TEXT)")
        .execute(store.pool())
        .await
        .expect("create table");

    // Simulate an existing indexed DB by committing a block (cursor now non-empty).
    commit_block(&store, &block(1, "0x1", "0x0", 1)).await;

    // Not fresh → defer_index must NOT drop; it ensures the index exists and
    // registers nothing (dropping a live table's index mid-follow is wrong).
    store
        .defer_index("t_k_idx", "CREATE INDEX IF NOT EXISTS t_k_idx ON t (k)")
        .await
        .expect("defer_index on resumed db");
    assert!(
        index_exists(&store, "t_k_idx").await,
        "index ensured on a resumed db"
    );
    assert_eq!(
        deferred_count(&store).await,
        0,
        "nothing registered on a resumed db"
    );

    drop_db("defer_idx_resume").await;
}

async fn apply_entity_block(store: &PgStore, height: u32, statements: &[&'static str]) {
    let mut tx = store.begin().await.unwrap();
    store.set_block_context(&mut tx, height).await.unwrap();
    for statement in statements {
        sqlx::raw_sql(*statement).execute(&mut *tx).await.unwrap();
    }
    store
        .set_cursor(
            &mut tx,
            &block(height, &format!("0x{height:x}"), "0xparent", 1),
        )
        .await
        .unwrap();
    store.commit(tx).await.unwrap();
}

async fn undo_count(store: &PgStore) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM palletide_entity_undo")
        .fetch_one(store.pool())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn entities_restore_insert_update_delete_and_primary_key_changes_after_restart() {
    let url = make_db("entity_restore").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE TABLE accounts (id TEXT PRIMARY KEY, balance NUMERIC NOT NULL); INSERT INTO accounts VALUES ('alice', 10), ('bob', 20)")
        .execute(store.pool()).await.unwrap();
    store.track_table("accounts").await.unwrap();
    store.track_table("accounts").await.unwrap();
    apply_entity_block(
        &store,
        1,
        &[
            "UPDATE accounts SET balance = 100 WHERE id = 'alice'",
            "DELETE FROM accounts WHERE id = 'bob'",
            "INSERT INTO accounts VALUES ('charlie', 30)",
        ],
    )
    .await;
    apply_entity_block(
        &store,
        2,
        &[
            "UPDATE accounts SET id = 'renamed', balance = 200 WHERE id = 'alice'",
            "UPDATE accounts SET balance = 300 WHERE id = 'charlie'",
        ],
    )
    .await;
    // Reconnect without registering again: both registration and undo survive.
    store.pool().close().await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    assert!(store.requires_block_context());
    store.rollback_to(1).await.unwrap();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, balance::text FROM accounts ORDER BY id")
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![
            ("alice".into(), "100".into()),
            ("charlie".into(), "30".into())
        ]
    );
    assert_eq!(store.cursor().await.unwrap().unwrap().number, 1);
    store.rollback_to(0).await.unwrap();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, balance::text FROM accounts ORDER BY id")
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![("alice".into(), "10".into()), ("bob".into(), "20".into())]
    );
    assert!(store.cursor().await.unwrap().is_none());
    assert_eq!(undo_count(&store).await, 0);
    store.rollback_to(0).await.unwrap(); // idempotent
    drop_db("entity_restore").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn entity_restore_failure_keeps_cursor_rows_and_undo_atomic() {
    let url = make_db("entity_atomic").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE TABLE accounts (id TEXT PRIMARY KEY, balance INT); INSERT INTO accounts VALUES ('alice', 10)")
        .execute(store.pool()).await.unwrap();
    store.track_table("accounts").await.unwrap();
    apply_entity_block(
        &store,
        1,
        &[
            "UPDATE accounts SET balance = 100 WHERE id = 'alice'",
            "INSERT INTO accounts VALUES ('orphan', 200)",
        ],
    )
    .await;
    // Inject a restoration failure after the orphan deletion would have run.
    sqlx::query("ALTER TABLE accounts ADD CHECK (balance >= 50)")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.rollback_to(0).await.is_err());
    assert_eq!(store.cursor().await.unwrap().unwrap().number, 1);
    assert_eq!(undo_count(&store).await, 2);
    let rows: Vec<(String, i32)> = sqlx::query_as("SELECT id, balance FROM accounts ORDER BY id")
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert_eq!(rows, vec![("alice".into(), 100), ("orphan".into(), 200)]);
    drop_db("entity_atomic").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn entity_undo_rejects_unattributed_writes_truncate_and_unavailable_history() {
    let url = make_db("entity_bounds").await;
    let store = PgStore::connect(StoreConfig::new(&url).with_reorg_retention(2))
        .await
        .unwrap();
    store.init().await.unwrap();
    sqlx::query("CREATE TABLE items (id INT PRIMARY KEY)")
        .execute(store.pool())
        .await
        .unwrap();
    store.track_table("items").await.unwrap();
    assert!(sqlx::query("INSERT INTO items VALUES (99)")
        .execute(store.pool())
        .await
        .is_err());
    assert!(sqlx::query("TRUNCATE items")
        .execute(store.pool())
        .await
        .is_err());
    for height in 1..=5 {
        let mut tx = store.begin().await.unwrap();
        store.set_block_context(&mut tx, height).await.unwrap();
        sqlx::query("INSERT INTO items VALUES ($1)")
            .bind(height as i32)
            .execute(&mut *tx)
            .await
            .unwrap();
        store
            .set_cursor(&mut tx, &block(height, "0xhash", "0xparent", 1))
            .await
            .unwrap();
        store.commit(tx).await.unwrap();
    }
    assert_eq!(undo_count(&store).await, 3);
    assert!(store
        .rollback_to(1)
        .await
        .unwrap_err()
        .to_string()
        .contains("pruned"));
    assert_eq!(store.cursor().await.unwrap().unwrap().number, 5);
    store.rollback_to(2).await.unwrap();
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM items ORDER BY id")
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert_eq!(ids, vec![1, 2]);
    drop_db("entity_bounds").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn registering_existing_data_cannot_claim_older_undo_history() {
    let url = make_db("entity_baseline").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    commit_block(&store, &block(5, "0x5", "0x4", 1)).await;
    sqlx::query("CREATE TABLE items (id INT PRIMARY KEY)")
        .execute(store.pool())
        .await
        .unwrap();
    store.track_table("items").await.unwrap();
    assert!(store
        .rollback_to(4)
        .await
        .unwrap_err()
        .to_string()
        .contains("tracking started"));
    assert_eq!(store.cursor().await.unwrap().unwrap().number, 5);
    drop_db("entity_baseline").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn entity_undo_handles_composite_keys_identity_generated_and_quoted_columns() {
    let url = make_db("entity_types").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE SCHEMA custom; CREATE TABLE custom.\"order\" (id BIGINT GENERATED ALWAYS AS IDENTITY, \"from\" TEXT, amount INT, doubled INT GENERATED ALWAYS AS (amount * 2) STORED, PRIMARY KEY(id, \"from\")); INSERT INTO custom.\"order\" (\"from\", amount) VALUES ('alice', 10)")
        .execute(store.pool()).await.unwrap();
    store.track_table("custom.\"order\"").await.unwrap();
    apply_entity_block(
        &store,
        1,
        &[
            "UPDATE custom.\"order\" SET amount = 20",
            "DELETE FROM custom.\"order\"",
        ],
    )
    .await;
    store.rollback_to(0).await.unwrap();
    let row: (i64, String, i32, i32) =
        sqlx::query_as("SELECT id, \"from\", amount, doubled FROM custom.\"order\"")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(row, (1, "alice".into(), 10, 20));
    drop_db("entity_types").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn aborted_entity_batch_leaves_no_rows_or_undo() {
    let url = make_db("entity_abort").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::query("CREATE TABLE items (id INT PRIMARY KEY)")
        .execute(store.pool())
        .await
        .unwrap();
    store.track_table("items").await.unwrap();
    let mut tx = store.begin().await.unwrap();
    store.set_block_context(&mut tx, 1).await.unwrap();
    sqlx::query("INSERT INTO items VALUES (1)")
        .execute(&mut *tx)
        .await
        .unwrap();
    store
        .set_cursor(&mut tx, &block(1, "0x1", "0x0", 1))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM items")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(undo_count(&store).await, 0);
    assert!(store.cursor().await.unwrap().is_none());
    drop_db("entity_abort").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn tracking_rejects_unsupported_tables_and_restores_deferrable_relations() {
    let url = make_db("entity_relations").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE TABLE no_key (id INT); CREATE TABLE parents (id INT PRIMARY KEY); CREATE TABLE children (id INT PRIMARY KEY, parent INT REFERENCES parents(id) DEFERRABLE INITIALLY DEFERRED); CREATE TABLE cascade_child (id INT PRIMARY KEY, parent INT REFERENCES parents(id) ON DELETE CASCADE)")
        .execute(store.pool()).await.unwrap();
    assert!(store
        .track_table("no_key")
        .await
        .unwrap_err()
        .to_string()
        .contains("primary key"));
    store.track_table("parents").await.unwrap();
    assert!(store.track_table("palletide_block").await.is_err());
    sqlx::query("DROP TABLE cascade_child")
        .execute(store.pool())
        .await
        .unwrap();
    store.track_table("parents").await.unwrap();
    store.track_table("children").await.unwrap();
    apply_entity_block(
        &store,
        1,
        &[
            "INSERT INTO parents VALUES (1)",
            "INSERT INTO children VALUES (1, 1)",
        ],
    )
    .await;
    apply_entity_block(&store, 2, &["DELETE FROM parents", "DELETE FROM children"]).await;
    store.rollback_to(1).await.unwrap();
    let parent: i32 = sqlx::query_scalar("SELECT parent FROM children")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(parent, 1);
    store.rollback_to(0).await.unwrap();
    let parents: i64 = sqlx::query_scalar("SELECT count(*) FROM parents")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(parents, 0);
    drop_db("entity_relations").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres"]
async fn cascaded_key_changes_and_deletes_restore_registered_children() {
    let url = make_db("cascade_restore").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE TABLE parents (id INT PRIMARY KEY); CREATE TABLE children (parent INT REFERENCES parents(id) ON UPDATE CASCADE ON DELETE CASCADE, id INT, PRIMARY KEY(parent,id)); INSERT INTO parents VALUES (1); INSERT INTO children VALUES (1, 9)")
        .execute(store.pool()).await.unwrap();
    store.track_table("parents").await.unwrap();
    store.track_table("children").await.unwrap();
    apply_entity_block(&store, 1, &["UPDATE parents SET id=2"]).await;
    apply_entity_block(&store, 2, &["DELETE FROM parents"]).await;
    store.rollback_to(1).await.unwrap();
    let parent: i32 = sqlx::query_scalar("SELECT parent FROM children")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(parent, 2);
    store.rollback_to(0).await.unwrap();
    let parent: i32 = sqlx::query_scalar("SELECT parent FROM children")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(parent, 1);
    assert_eq!(undo_count(&store).await, 0);
    drop_db("cascade_restore").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres"]
async fn partition_root_tracks_row_movement_and_rejects_volatile_tables() {
    let url = make_db("partition_restore").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql("CREATE TABLE events (height INT, id TEXT, amount INT, PRIMARY KEY(height,id)) PARTITION BY RANGE(height); CREATE TABLE events_low PARTITION OF events FOR VALUES FROM (0) TO (10); CREATE TABLE events_high PARTITION OF events FOR VALUES FROM (10) TO (20); CREATE UNLOGGED TABLE volatile_data (id INT PRIMARY KEY)")
        .execute(store.pool()).await.unwrap();
    assert!(store.track_table("volatile_data").await.is_err());
    assert!(store.track_table("events_low").await.is_err());
    store.track_table("events").await.unwrap();
    apply_entity_block(&store, 1, &["INSERT INTO events VALUES (1,'a',10)"]).await;
    apply_entity_block(
        &store,
        2,
        &["UPDATE events SET height=11, amount=20 WHERE id='a'"],
    )
    .await;
    store.rollback_to(1).await.unwrap();
    let row: (i32, i32) = sqlx::query_as("SELECT height,amount FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(row, (1, 10));
    store.rollback_to(0).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    drop_db("partition_restore").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres"]
async fn schema_checkpoint_is_atomic_and_later_history_remains_reversible() {
    let url = make_db("schema_checkpoint").await;
    let store = PgStore::connect(StoreConfig::new(&url)).await.unwrap();
    store.init().await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE items (id INT PRIMARY KEY, amount INT); INSERT INTO items VALUES(1,10)",
    )
    .execute(store.pool())
    .await
    .unwrap();
    store.track_table("items").await.unwrap();
    apply_entity_block(&store, 1, &["UPDATE items SET amount=20"]).await;
    assert!(store
        .apply_schema_change("ALTER TABLE items ADD COLUMN note TEXT; SELECT 1/0")
        .await
        .is_err());
    assert_eq!(undo_count(&store).await, 1);
    store.rollback_to(0).await.unwrap();
    apply_entity_block(&store, 2, &["UPDATE items SET amount=20"]).await;
    store
        .apply_schema_change("ALTER TABLE items ADD COLUMN note TEXT")
        .await
        .unwrap();
    assert_eq!(undo_count(&store).await, 0);
    apply_entity_block(&store, 3, &["UPDATE items SET note='new', amount=30"]).await;
    store.rollback_to(2).await.unwrap();
    let row: (i32, Option<String>) = sqlx::query_as("SELECT amount,note FROM items")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(row, (20, None));
    assert!(store.rollback_to(1).await.is_err());
    drop_db("schema_checkpoint").await;
}
