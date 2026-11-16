# Configuration & operations reference

Every knob palletide exposes, in one place: the Rust config structs, the env/TOML
layer that fills them, the data-source trade-offs, and the metrics it exports.

For the step-by-step build walkthrough, see the
[**GUIDE**](https://github.com/kunal171/palletide/blob/main/docs/GUIDE.md). For the rendered docs site, see
[kunal171.github.io/palletide](https://kunal171.github.io/palletide/).

---

## The config structs

Each component takes a typed config. You can build these by hand, or let
[`palletide-config`](https://github.com/kunal171/palletide/blob/main/crates/palletide-config) fill them from TOML + env (below).

| Component | Type | Key options |
|---|---|---|
| Source | `SourceConfig` | `url` (WSS endpoint), `batch_size` (default 100), `concurrency` (default 16), `selection` (`DataSelection` — fetch only the events/extrinsics you use), `retry` (`RetryConfig` — 5 retries, 250ms doubling to a 30s cap), `ss58_prefix` (default 42), `strict` (make per-item decode failures hard errors; default off) |
| Store | `StoreConfig` | `url` (Postgres), `max_connections` (default 5), `reorg_retention` (block rows kept for reorg detection; older rows pruned on commit — default 5000, `0` = keep all) |
| Processor | `ProcessorConfig` | `start_height`, `batch_size` (default 100), `max_reorg_depth` (bound the rewind on a reorg — default 64, `0` = unbounded) |
| GraphQL | `GraphqlConfig` | `addr` (default `0.0.0.0:4350`), `path` (default `/graphql`) |

## The env / TOML layer

`IndexerConfig::load()` reads an optional `palletide.toml` (`[source]` / `[store]` /
`[processor]` tables), overlays environment variables, and auto-loads a local
`.env`. `WS_URL` and `DATABASE_URL` are the only required values.

| Env var | TOML key | Meaning |
|---|---|---|
| `WS_URL` | `source.url` | **Required.** Chain WebSocket RPC endpoint. |
| `DATABASE_URL` | `store.url` | **Required.** Postgres connection string. |
| `BATCH_SIZE` | `source.batch_size` + `processor.batch_size` | Blocks per fetch / per commit. |
| `CONCURRENCY` | `source.concurrency` | In-flight block fetches (hides RPC latency). |
| `SS58_PREFIX` | `source.ss58_prefix` | Address format (42 = Substrate `5…`, 0 = Polkadot `1…`). |
| `STRICT` | `source.strict` | `1` = a decode failure aborts the block, instead of recording an empty value. |
| `REORG_RETENTION` | `store.reorg_retention` | `(height, hash)` rows kept for reorg checks. |
| `START_HEIGHT` | `processor.start_height` | Backfill start on a fresh DB. |
| `MAX_REORG_DEPTH` | `processor.max_reorg_depth` | Bound on how far a reorg may rewind. |

The example binaries add three app-level knobs: `SERVE` (serve the GraphQL API),
`FOLLOW` (follow the tip after backfill), and `GRAPHQL_PORT` — plus `RUST_LOG`.

---

## Data sources

The engine talks to any `DataSource`. Three ship in
[`palletide-source`](https://github.com/kunal171/palletide/blob/main/crates/palletide-source):

| Source | Backfill | Live tip | Use when |
|---|---|---|---|
| **`SubxtSource`** (default) | slow (RPC) | ✅ | Any Substrate chain; getting started; live indexing |
| **`SqdPortalSource`** (`sqd` feature) | **fast** (columnar) | ❌ | Large historical backfill on a chain with an SQD dataset |
| **`HybridSource`** (`sqd` feature) | **fast** | ✅ | Production: fast catch-up *and* live follow |

**`SubxtSource`** talks RPC over WebSocket and decodes each block against **its
own** spec-version metadata, so a backfill across a runtime upgrade stays correct
with no per-chain codegen. It is latency-bound — throughput is capped by the node
(tens of blocks/sec against a public endpoint).

**`SqdPortalSource`** serves pre-decoded, columnar, batched history from the
[SQD portal](https://docs.sqd.dev), far faster than per-block RPC. Because it is
columnar, **throughput is dominated by how much you fetch**: a narrow field
selection with large batches is dramatically faster than pulling full block data
(see [RFC 024](https://github.com/kunal171/palletide/blob/main/docs/rfcs/024-sqd-portal-source.md) for measured numbers). Two
caveats, both inherent to the portal:

- **Backfill-only.** It has no live Substrate tip, so `next_finalized` errors on
  its own — pair it with an RPC source via `HybridSource`.
- **Decoded values are equivalent, not identical, to RPC.** The portal pre-decodes
  args to JSON; palletide bridges that to `scale_value::Value` so handlers keep the
  same type. Structural fields (heights, hashes, event names/indices, timestamps)
  match RPC exactly; complex arg shapes (enums, byte arrays) may differ.

**`HybridSource`** is the production shape — backfill from the portal, follow the
tip over RPC:

```rust
use palletide_source::{HybridSource, SqdConfig, SqdPortalSource, SourceConfig, SubxtSource};

let portal = SqdPortalSource::connect(
    SqdConfig::new("https://portal.sqd.dev", "polkadot").with_batch_size(5000),
)?;
let rpc = SubxtSource::connect(SourceConfig::new("wss://your-node:9944")).await?;
let source = HybridSource::new(portal, rpc); // fast backfill → live RPC tip
```

It is generic over any backfill + tip `DataSource`, not just portal + RPC. Because
everything is a `DataSource`, **handlers and the engine don't change** when you
switch — that's the point of the trait seam.

---

## Observability

The engine exposes its run loop through a lightweight `ProcessorObserver` hook — a
synchronous, backend-agnostic trait called at key points: `on_batch_committed`
(cursor, block/event counts, commit time), `on_reorg` (fork height + depth),
`on_head` (new finalized tip), `on_fetch` (fetch latency), and `on_error`. Every
method has a no-op default, so the default `NoopObserver` costs nothing.

```rust
let processor = Processor::new(source, store, handlers, config)
    .with_observer(my_observer); // Arc<dyn ProcessorObserver>
```

Use it for a progress/ETA reporter, a test spy, or your own dashboard feed.

### Prometheus metrics

Enable the `metrics` feature for a ready-made observer and a `/metrics` endpoint:

```toml
palletide = { version = "...", features = ["metrics"] }
```

```rust
use palletide::{install_prometheus, PrometheusObserver};
use std::sync::Arc;

install_prometheus("0.0.0.0:9000".parse()?)?; // serves /metrics
let processor = Processor::new(source, store, handlers, config)
    .with_observer(Arc::new(PrometheusObserver::new()));
```

Exported series:

| Kind | Series |
|---|---|
| Gauges | `palletide_cursor_height`, `palletide_finalized_head`, `palletide_head_lag` |
| Counters | `palletide_blocks_processed_total`, `palletide_events_decoded_total`, `palletide_reorgs_total`, `palletide_errors_total`, `palletide_decode_failures_total` |
| Histograms | `palletide_reorg_depth`, `palletide_batch_commit_seconds`, `palletide_fetch_seconds` |

The feature is off by default — no metrics dependencies are compiled unless you
ask for them.

> The source emits `palletide_decode_failures_total` (labelled by `kind` / `pallet`)
> whenever an event's fields or an extrinsic's args fail to decode. By default the
> item is recorded with an empty value and indexing continues; set
> `SourceConfig.strict` to make such a failure a hard error instead (useful in CI
> to catch metadata drift rather than silently write empty data).

### Health endpoints

The GraphQL server mounts two probe routes alongside the API, for load balancers
and orchestrators:

| Route | Meaning | Use for |
|---|---|---|
| `GET /healthz` | The process is up and serving. Does **no** I/O — always 200. | Liveness (k8s `livenessProbe`, ECS container check) |
| `GET /readyz` | The database is reachable. 200 with `{"status":"ready","height":…,"indexed_blocks":…}`, or 503 `{"status":"unavailable",…}`. | Readiness (ALB/target-group health, k8s `readinessProbe`) |

`/healthz` deliberately ignores the database: a liveness probe that fails on a
transient Postgres blip would get the container killed and restarted, which does
not fix a database. Point the restart-triggering probe at `/healthz` and the
traffic-gating one at `/readyz`.

`/readyz` needs the pool, so it comes from a separate constructor:

```rust
use palletide_graphql::{build_status_schema, router_with_health, GraphqlConfig};

// router(..)             → GraphQL + /healthz
// router_with_health(..) → GraphQL + /healthz + /readyz
let app = router_with_health(schema, &config, pool);
```

Probe them by hand with the bundled example:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:55432/palletide \
  cargo run -p palletide-graphql --example healthprobe
curl -i localhost:4350/readyz
```

> Container probes: the runtime image is `debian-slim` with **no `curl`**, and
> `/bin/sh` is dash (no `/dev/tcp`). A compose/ECS shell probe must invoke `bash`
> explicitly — see the healthcheck in
> [`docker-compose.yml`](https://github.com/kunal171/palletide/blob/main/docker-compose.yml). Prefer an ALB/target-group HTTP
> check where you have one; it needs nothing inside the image.

---

## Reorgs & finality

The processor anchors on the chain's **finalized** head. Before committing a block
it validates that the block's `parent_hash` matches the hash stored for the
previous height:

- **Match** → commit normally (handler writes + cursor advance, atomically).
- **Mismatch** → a reorg replaced the parent. The processor walks down to the
  **true common ancestor** (comparing stored hashes against the source's canonical
  hashes), rolls back the diverged tail in one pass, and re-fetches from the fork
  point. The rewind is bounded by `max_reorg_depth` (default 64; `0` = unbounded) —
  a deeper fork errors rather than rewinding unboundedly.

Because palletide indexes finalized blocks, deep reorgs are not expected; the
parent-hash check protects against any divergence within the retained window. On
GRANDPA chains the finalized cursor is clean and unambiguous.

---

## Testing

```bash
# Fast, offline unit tests (no chain, no database):
cargo test

# Lint:
cargo clippy --workspace --all-targets

# Network/DB integration tests are #[ignore]d so offline/CI runs stay green.
# Run them explicitly with a chain + Postgres available:
docker run -d --name pg -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=palletide \
    -p 55432:5432 postgres:16-alpine

PALLETIDE_TEST_WS=wss://your-substrate-node:9944 \
PALLETIDE_TEST_DB=postgres://postgres:postgres@localhost:55432/palletide \
    cargo test --workspace -- --ignored
```

Integration tests cover: decoding real mainnet blocks (`palletide-source`), the full
store lifecycle incl. reorg rollback (`palletide-store`), an end-to-end
mainnet→Postgres run (`palletide`), and serving GraphQL over HTTP (`palletide-graphql`).
