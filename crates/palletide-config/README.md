# palletide-config

Typed configuration loading for Palletide indexers.

## Install

```bash
cargo add palletide-config@0.5.1
```

## Usage

`IndexerConfig::load()` reads an optional TOML file and environment variables,
validates the inputs, and builds source, store, and processor configuration.
Environment variables override file values; a local `.env` file is also supported.

Set `WS_URL` and `DATABASE_URL` to connect to your Substrate RPC and PostgreSQL.
Optional settings include batch size, concurrency, SS58 prefix, request timeout,
fallback RPC URLs, start height, and reorg retention.

Use `source_config()`, `store_config()`, and `processor_config()` on the loaded
configuration to construct the corresponding framework components.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-config) · [Repository](https://github.com/kunal171/palletide)
