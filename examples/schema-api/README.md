# Generated schema API

A compiled schema-first example covering keyword fields, large numbers, forward
and reverse relations, indexed filters, keyset pagination, bulk writes and undo.
The generated artifacts are committed so every native platform compiles them.

Regenerate from the repository root:

```bash
cargo run -p palletide-codegen -- generate examples/schema-api/schema.graphql --out examples/schema-api/src/generated
```

Run the PostgreSQL integration test:

```bash
PALLETIDE_TEST_DB=postgres://postgres:postgres@localhost:55432/palletide cargo test -p palletide-example-schema-api --test read_api -- --ignored
```

Benchmark generated serial and bulk upserts with journaling enabled:

```bash
PALLETIDE_TEST_DB=postgres://postgres:postgres@localhost:55432/palletide cargo run -p palletide-example-schema-api --example bulk_bench -- 1000
```

Both use throwaway databases. The benchmark rolls each write transaction back
and checks that one undo entry exists per inserted row in both modes. Throughput
depends on database/network latency, hardware, and table layout; it is not a
chain-ingestion benchmark.
