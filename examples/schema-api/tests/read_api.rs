use async_graphql::{EmptyMutation, EmptySubscription, Request, Schema, Variables};
use palletide::Store;
use palletide_example_schema_api::{
    generated::entities::{register_entities, Account, Transfer, Value as StoredValue},
    QueryRoot,
};
use palletide_store::{PgStore, StoreConfig};
use sqlx::{Connection, PgConnection};

#[tokio::test]
#[ignore = "database: needs Postgres"]
async fn generated_queries_bulk_writes_and_rollback_work_end_to_end() {
    let admin = std::env::var("PALLETIDE_TEST_DB")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:55432/palletide".into());
    let database = format!("palletide_schema_api_{}", std::process::id());
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
    store.init().await.unwrap();
    sqlx::raw_sql(include_str!("../src/generated/migrations/0001_schema.sql"))
        .execute(store.pool())
        .await
        .unwrap();
    register_entities(&store).await.unwrap();
    let mut tx = store.begin().await.unwrap();
    store.set_block_context(&mut tx, 1).await.unwrap();
    Account::upsert_many(
        &[Account {
            id: "alice".into(),
            name: "Alice".into(),
            r#type: Some("person".into()),
            balance: "100000000000000000001".into(),
        }],
        &mut tx,
    )
    .await
    .unwrap();
    let rows: Vec<_> = (1..=4)
        .map(|i| Transfer {
            id: format!("t{i}"),
            owner: "alice".into(),
            featured_by: if i == 1 { Some("alice".into()) } else { None },
            amount: format!("10000000000000000000{i}"),
            score: if i == 4 { None } else { Some(i) },
            tags: vec!["tag".into()],
            amounts: vec!["100000000000000000001".into()],
            meta: Some(serde_json::json!({"i":i})),
        })
        .collect();
    Transfer::upsert_many(&rows, &mut tx).await.unwrap();
    StoredValue::upsert_many(
        &[StoredValue {
            id: "value-1".into(),
            label: Some("quoted table".into()),
        }],
        &mut tx,
    )
    .await
    .unwrap();
    store.commit(tx).await.unwrap();
    let schema = Schema::build(QueryRoot::default(), EmptyMutation, EmptySubscription)
        .data(store.pool().clone())
        .finish();
    let response = schema.execute("{ values { label } valuesCount accounts { id name type transfers(limit: 10) { id owner { name } amounts } featured { id } } }").await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    let data = response.data.into_json().unwrap();
    assert_eq!(data["accounts"][0]["type"], "person");
    assert_eq!(
        data["accounts"][0]["transfers"].as_array().unwrap().len(),
        4
    );
    assert_eq!(data["accounts"][0]["featured"]["id"], "t1");
    assert_eq!(
        data["accounts"][0]["transfers"][0]["amounts"][0],
        "100000000000000000001"
    );
    let query = "query($after: String) { transfers(limit: 2, orderBy: \"amount\", descending: false, after: $after, filters: [{field: \"amount\", gt: \"100000000000000000000\"}]) { id amount _cursor(orderBy: \"amount\", descending: false) } }";
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    let data = response.data.into_json().unwrap();
    assert_eq!(data["transfers"][0]["id"], "t1");
    let cursor = data["transfers"][1]["_cursor"].as_str().unwrap();
    let response = schema
        .execute(
            Request::new(query)
                .variables(Variables::from_json(serde_json::json!({"after":cursor}))),
        )
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    let data = response.data.into_json().unwrap();
    assert_eq!(data["transfers"][0]["id"], "t3");
    assert_eq!(data["transfers"][1]["id"], "t4");
    let invalid = schema
        .execute("{ transfers(orderBy: \"id; DROP TABLE transfers\") { id } }")
        .await;
    assert!(!invalid.errors.is_empty());
    let query = "query($after: String) { transfers(limit: 3, orderBy: \"score\", descending: false, after: $after) { id _cursor(orderBy: \"score\", descending: false) } }";
    let first = schema.execute(query).await;
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    let data = first.data.into_json().unwrap();
    let cursor = data["transfers"][2]["_cursor"].as_str().unwrap();
    let last = schema
        .execute(
            Request::new(query)
                .variables(Variables::from_json(serde_json::json!({"after":cursor}))),
        )
        .await;
    assert!(last.errors.is_empty(), "{:?}", last.errors);
    assert_eq!(last.data.into_json().unwrap()["transfers"][0]["id"], "t4");
    store.rollback_to(0).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM transfers")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    store.pool().close().await;
    let mut conn = PgConnection::connect(&admin).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&mut conn)
    .await
    .unwrap();
}
