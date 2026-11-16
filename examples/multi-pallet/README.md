# palletide example: `multi-pallet`

A runnable [`palletide`](../../) indexer that indexes **two pallets with two
handlers** into two tables — the common real-world shape the single-handler
[`transfers`](../transfers) example doesn't cover.

It shows:

- **Two independent `Handler`s** on one `Processor`:
  - [`BalancesHandler`](./src/balances.rs) — `Balances.Transfer` → `balance_transfers` table.
  - [`AssetsHandler`](./src/assets.rs) — `Assets.Created` / `Assets.Destroyed` → `asset_lifecycle` table.
- **Atomic multi-handler commit.** Both handlers' writes and the cursor advance
  go on the **same transaction** per batch, so a block is either fully indexed
  across both tables or not at all — a crash never leaves one table ahead of the
  other.
- **Two decode styles.** `BalancesHandler` uses the simple per-block
  `process_block`; `AssetsHandler` overrides **`process_batch`** to accumulate the
  whole batch and **bulk-write** it in one multi-row `INSERT` (the high-throughput
  pattern).
- **GraphQL over both tables** plus the framework's `indexerStatus`, from the one
  Postgres pool the indexer writes to.
- **Config via [`palletide-config`](../../crates/palletide-config)** — no hand-rolled
  env parsing.
- **Versioned handler migrations.** `AssetsHandler` owns its schema via
  [`migrations/assets/`](./migrations/assets) (embedded with `sqlx::migrate!`) and
  applies them in `init` with `store.run_handler_migrations(&MIGRATOR, name)` —
  applied once, in order, tracked in `_sqlx_migrations_assets`, isolated from the
  framework's own migrations. `BalancesHandler` keeps the simpler ad-hoc
  `CREATE TABLE IF NOT EXISTS` for contrast.

## Run

```bash
# A Postgres to index into
docker run -d --name palletide-db \
    -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=palletide \
    -p 55432:5432 postgres:16-alpine

WS_URL=wss://your-substrate-node:9944 \
DATABASE_URL=postgres://postgres:postgres@localhost:55432/palletide \
    cargo run -p palletide-example-multi-pallet
```

Then open <http://localhost:4350/graphql> and try:

```graphql
{
  balanceTransfers(limit: 5) { blockHeight fromAccount toAccount amount }
  assetEvents(limit: 5)      { blockHeight action assetId owner }
  indexerStatus              { cursorHeight }
}
```

## Configuration

Framework config (source / store / processor) is loaded by `palletide-config` —
env vars and an optional `palletide.toml` (see [`palletide.toml.example`](../../palletide.toml.example)).
`WS_URL` and `DATABASE_URL` are required; everything else has defaults.

This binary adds three example-app knobs: `FOLLOW` (default `1`), `SERVE`
(default `1`), `GRAPHQL_PORT` (default `4350`).

## Tables

```
balance_transfers(block_height, event_index, from_account, to_account, amount,
                  UNIQUE(block_height, event_index))
asset_lifecycle(block_height, event_index, action, asset_id, owner,
                UNIQUE(block_height, event_index))
```

The `UNIQUE (block_height, event_index)` constraint makes re-indexing idempotent
in both tables.
