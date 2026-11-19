//! Compare serial upserts and generated bulk upserts with undo capture enabled.
//! PALLETIDE_TEST_DB=postgres://... cargo run -p palletide-example-schema-api --example bulk_bench -- 1000
use palletide::Store;
use palletide_example_schema_api::generated::entities::{register_entities, Account};
use palletide_store::{PgStore, StoreConfig};
use sqlx::{Connection, PgConnection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let admin = std::env::var("PALLETIDE_TEST_DB")?;
    let count = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1000".into())
        .parse::<usize>()?
        .clamp(1, 100_000);
    let database = format!("palletide_bulk_bench_{}", std::process::id());
    let mut connection = PgConnection::connect(&admin).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    let url = format!(
        "{}/{}",
        admin.rsplit_once('/').ok_or("invalid database URL")?.0,
        database
    );
    let store = PgStore::connect(StoreConfig::new(url)).await?;
    store.init().await?;
    sqlx::raw_sql(include_str!("../src/generated/migrations/0001_schema.sql"))
        .execute(store.pool())
        .await?;
    register_entities(&store).await?;
    let rows: Vec<_> = (0..count)
        .map(|i| Account {
            id: format!("{i:08}"),
            name: format!("Account {i}"),
            r#type: None,
            balance: "100000000000000000001".into(),
        })
        .collect();
    for bulk in [false, true] {
        let mut tx = store.begin().await?;
        store.set_block_context(&mut tx, 1).await?;
        let start = std::time::Instant::now();
        if bulk {
            Account::upsert_many(&rows, &mut tx).await?;
        } else {
            for row in &rows {
                row.upsert(&mut *tx).await?;
            }
        }
        let elapsed = start.elapsed();
        let undo: i64 = sqlx::query_scalar("SELECT count(*) FROM palletide_entity_undo")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(undo, count as i64);
        println!(
            "{}: {} rows, {:.3}s, {:.0} rows/s, {} undo entries",
            if bulk { "bulk" } else { "serial" },
            count,
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64(),
            undo
        );
        tx.rollback().await?;
    }
    store.pool().close().await;
    let mut connection = PgConnection::connect(&admin).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&mut connection)
    .await?;
    Ok(())
}
