# Quickstart

The fastest way to see palletide work is the bundled
[`transfers`](https://github.com/kunal171/palletide/tree/main/examples/transfers)
example, which indexes `Assets.Deposited` / `Assets.Withdrawn` events into
Postgres and serves them over GraphQL.

**Prerequisites:** Rust ≥ 1.96, Docker (for Postgres).

```bash
# 1. A Postgres to index into
docker run -d --name palletide-db \
    -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=palletide \
    -p 55432:5432 postgres:16-alpine

# 2. Configure (WS_URL + DATABASE_URL are required; a local .env is auto-loaded)
cp examples/transfers/.env.example .env
#   edit WS_URL to point at a chain, e.g. wss://rpc.polkadot.io

# 3. Run the indexer (backfills ~20 recent blocks, then follows the tip).
cargo run -p palletide-example-transfers
```

Then query what it indexed (in `psql` or any client →
`postgres://postgres:postgres@localhost:55432/palletide`):

```sql
SELECT direction, count(*) FROM transfers GROUP BY direction;

SELECT block_height, direction, asset_id, account, amount
FROM transfers ORDER BY block_height DESC LIMIT 10;
```

Accounts are rendered as **SS58** (`5…`) addresses, just like block explorers.

## Install the CLI

Requires Rust 1.96 or newer. Install from GitHub until the renamed crates are
published on crates.io:

```bash
cargo install --git https://github.com/kunal171/palletide --locked palletide-codegen
palletide-codegen --help
```

From a local checkout, use `cargo install --path crates/palletide-codegen --locked`.
You can also run `cargo run -p palletide-codegen -- <command>` without installing.
If the installed command is not found, add Cargo's binary directory (usually
`~/.cargo/bin`) to your `PATH`.

## Two example shapes

- [`transfers`](https://github.com/kunal171/palletide/tree/main/examples/transfers) —
  a single hand-written handler, one event → one table.
- [`multi-pallet`](https://github.com/kunal171/palletide/tree/main/examples/multi-pallet)
  — two pallets, two handlers (one bulk-writing via the two-phase path), committing
  **atomically** into two tables and served over GraphQL.

## Next

- Prefer a clone-and-edit starting point? See the
  [starter template](./starter-template.md).
- Ready to index your own chain? Follow the
  [full guide](./guide.md).
