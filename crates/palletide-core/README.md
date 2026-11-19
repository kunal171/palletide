# palletide-core

Core traits and chain-agnostic types for Palletide, a Rust framework for indexing Substrate chains.

## Install

```bash
cargo add palletide-core@0.5.1
```

## Usage

- `DataSource` supplies historical batches and finalized blocks.
- `Handler` maps blocks and events into application data.
- `Store` commits application writes and chain progress together.
- `ProcessorObserver` exposes progress and reorg notifications.

Use this crate when implementing a custom source, handler, store, or observer.
For the indexing engine, use [`palletide`](https://crates.io/crates/palletide).

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-core) · [Repository](https://github.com/kunal171/palletide)
