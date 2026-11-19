# palletide-source

Substrate block sources and decoded-value helpers for the Palletide indexing framework.

## Install

```bash
cargo add palletide-source@0.5.1
```

## Usage

`SubxtSource` reads blocks, events, and extrinsics over RPC using each block's
runtime metadata. `RpcSource<T>` supports custom Subxt chain configurations.
Source configuration supports request timeouts, retries, and fallback endpoints.

Enable the `sqd` feature for `SqdPortalSource` and `HybridSource`, which combine
portal backfill with RPC following. Shared helpers extract balances, account
addresses, and other fields from decoded values.

Requires Rust 1.96 or newer.

[Guide](https://kunal171.github.io/palletide/) · [API documentation](https://docs.rs/palletide-source) · [Repository](https://github.com/kunal171/palletide)
