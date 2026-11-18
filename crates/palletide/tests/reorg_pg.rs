//! Database-only reorg tests: no chain RPC is needed.
use async_trait::async_trait;
use palletide::{
    Block, BlockBatch, BlockId, DataSource, Handler, Processor, ProcessorConfig, Result, Store,
};
use palletide_store::{PgStore, StoreConfig};
use sqlx::{Connection, PgConnection};
use std::sync::{Arc, Mutex};

struct ForkSource {
    canonical: Vec<Block>,
    tip: Mutex<Option<BlockBatch>>,
}

#[async_trait]
impl DataSource for ForkSource {
    async fn finalized_head(&self) -> Result<u32> {
        Ok(self.canonical.last().unwrap().id.number)
    }
    async fn fetch_batch(&self, from: u32, to: u32) -> Result<BlockBatch> {
        Ok(BlockBatch {
            blocks: self
                .canonical
                .iter()
                .filter(|b| (from..=to).contains(&b.id.number))
                .cloned()
                .collect(),
        })
    }
    async fn next_finalized(&self) -> Result<BlockBatch> {
        Ok(self
            .tip
            .lock()
            .unwrap()
            .take()
            .unwrap_or(BlockBatch { blocks: vec![] }))
    }
    fn name(&self) -> &str {
        "fork-fixture"
    }
}

struct Projection;
#[async_trait]
impl Handler<PgStore> for Projection {
    async fn init(&self, store: &PgStore) -> Result<()> {
        sqlx::raw_sql("CREATE TABLE accounts (id TEXT PRIMARY KEY, balance INT); CREATE TABLE events (id TEXT PRIMARY KEY)")
            .execute(store.pool()).await.map_err(|e| palletide::PalletideError::Handler(e.to_string()))?;
        store.track_table("accounts").await?;
        store.track_table("events").await
    }
    async fn process_block<'a>(
        &self,
        b: &Block,
        tx: &mut <PgStore as Store>::Tx<'a>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO accounts VALUES ('alice', $1) ON CONFLICT (id) DO UPDATE SET balance = EXCLUDED.balance")
            .bind(b.id.number as i32 * 10).execute(&mut **tx).await
            .map_err(|e| palletide::PalletideError::Handler(e.to_string()))?;
        sqlx::query("INSERT INTO events VALUES ($1)")
            .bind(&b.id.hash)
            .execute(&mut **tx)
            .await
            .map_err(|e| palletide::PalletideError::Handler(e.to_string()))?;
        Ok(())
    }
    fn name(&self) -> &str {
        "projection"
    }
}

fn block(height: u32, branch: &str, parent: &str) -> Block {
    Block {
        id: BlockId {
            number: height,
            hash: format!("{height}{branch}"),
        },
        parent_hash: parent.into(),
        timestamp: None,
        spec_version: 1,
        finalized: true,
        events: vec![],
        extrinsics: vec![],
    }
}

async fn setup(
    suffix: &str,
    canonical: Vec<Block>,
    tip: Option<Block>,
) -> (Processor<ForkSource, PgStore>, String) {
    let admin = std::env::var("PALLETIDE_TEST_DB")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:55432/palletide".into());
    let database = format!("palletide_reorg_{suffix}");
    let mut conn = PgConnection::connect(&admin).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {database} WITH (FORCE)"
    )))
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();
    let url = format!("{}/{}", admin.rsplit_once('/').unwrap().0, database);
    let store = PgStore::connect(StoreConfig::new(url)).await.unwrap();
    let processor = Processor::new(
        ForkSource {
            canonical,
            tip: Mutex::new(tip.map(|b| BlockBatch { blocks: vec![b] })),
        },
        store,
        vec![Arc::new(Projection)],
        ProcessorConfig::from_height(1),
    );
    processor.init().await.unwrap();
    (processor, admin)
}

async fn cleanup(processor: Processor<ForkSource, PgStore>, admin: &str, suffix: &str) {
    processor.store().pool().close().await;
    drop(processor);
    let mut conn = PgConnection::connect(admin).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE palletide_reorg_{suffix} WITH (FORCE)"
    )))
    .execute(&mut conn)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn reorg_inside_a_committed_batch_restores_the_exact_fork_state() {
    let first = block(1, "a", "0");
    let second = block(2, "a", "1a");
    let (processor, admin) = setup(
        "batch",
        vec![
            first.clone(),
            second.clone(),
            block(3, "b", "2a"),
            block(4, "b", "3b"),
        ],
        None,
    )
    .await;
    processor
        .process_batch_blocks(&[first, second, block(3, "a", "2a")])
        .await
        .unwrap();
    assert_eq!(
        processor
            .process_batch_blocks(&[block(4, "b", "3b")])
            .await
            .unwrap(),
        Some(3)
    );
    let balance: i32 = sqlx::query_scalar("SELECT balance FROM accounts")
        .fetch_one(processor.store().pool())
        .await
        .unwrap();
    assert_eq!(
        balance, 20,
        "restore updates at the exact fork, not the batch boundary"
    );
    let events: Vec<String> = sqlx::query_scalar("SELECT id FROM events ORDER BY id")
        .fetch_all(processor.store().pool())
        .await
        .unwrap();
    assert_eq!(events, vec!["1a", "2a"]);
    processor.backfill().await.unwrap();
    let events: Vec<String> = sqlx::query_scalar("SELECT id FROM events ORDER BY id")
        .fetch_all(processor.store().pool())
        .await
        .unwrap();
    assert_eq!(events, vec!["1a", "2a", "3b", "4b"]);
    assert_eq!(
        processor.store().cursor().await.unwrap().unwrap().hash,
        "4b"
    );
    cleanup(processor, &admin, "batch").await;
}

#[tokio::test]
#[ignore = "database: needs Postgres; run with --ignored"]
async fn live_reorg_refetches_missing_blocks_before_advancing_to_tip() {
    let first = block(1, "a", "0");
    let (processor, admin) = setup(
        "live",
        vec![
            first.clone(),
            block(2, "b", "1a"),
            block(3, "b", "2b"),
            block(4, "b", "3b"),
        ],
        Some(block(4, "b", "3b")),
    )
    .await;
    processor
        .process_batch_blocks(&[first, block(2, "a", "1a"), block(3, "a", "2a")])
        .await
        .unwrap();
    processor.follow(Some(1)).await.unwrap();
    let events: Vec<String> = sqlx::query_scalar("SELECT id FROM events ORDER BY id")
        .fetch_all(processor.store().pool())
        .await
        .unwrap();
    assert_eq!(events, vec!["1a", "2b", "3b", "4b"]);
    assert_eq!(processor.store().cursor().await.unwrap().unwrap().number, 4);
    cleanup(processor, &admin, "live").await;
}
#[tokio::test]
#[ignore = "database: needs Postgres"]
async fn pruned_history_is_not_assumed_to_be_a_common_ancestor() {
    let canonical = vec![
        block(1, "a", "0"),
        block(2, "b", "1a"),
        block(3, "b", "2b"),
        block(4, "b", "3b"),
        block(5, "b", "4b"),
        block(6, "b", "5b"),
    ];
    let (initial, admin) = setup("pruned_audit", canonical.clone(), None).await;
    let store = initial.store().clone().with_reorg_retention(2);
    drop(initial);
    let p = Processor::new(
        ForkSource {
            canonical,
            tip: Mutex::new(None),
        },
        store,
        vec![Arc::new(Projection)],
        ProcessorConfig::from_height(1).with_max_reorg_depth(0),
    );
    p.process_batch_blocks(&[
        block(1, "a", "0"),
        block(2, "a", "1a"),
        block(3, "a", "2a"),
        block(4, "a", "3a"),
        block(5, "a", "4a"),
    ])
    .await
    .unwrap();
    let result = p.process_batch_blocks(&[block(6, "b", "5b")]).await;
    let events: Vec<String> = sqlx::query_scalar("SELECT id FROM events ORDER BY id")
        .fetch_all(p.store().pool())
        .await
        .unwrap();
    let diagnostic = format!(
        "result={:?}; remaining events={:?}; cursor={:?}",
        result,
        events,
        p.store().cursor().await.unwrap()
    );
    cleanup(p, &admin, "pruned_audit").await;
    assert!(
        result.is_err(),
        "unknown history accepted as ancestor: {}",
        diagnostic
    );
}
