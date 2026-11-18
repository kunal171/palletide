//! [`SubxtSource`]: a [`DataSource`] backed by a direct WebSocket RPC connection
//! to a Substrate node, via `subxt`. Works against any Substrate chain.

use crate::config::SourceConfig;
use crate::mapping::map_block;
use crate::retry::retry_async;
use async_trait::async_trait;
use futures::lock::Mutex;
use palletide_core::{BlockBatch, BlockNumber, DataSource, PalletideError, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use subxt::config::PolkadotConfig;
use subxt::{Config, OnlineClient};

/// The subxt `Config` the source uses. `PolkadotConfig` matches the common
/// Substrate defaults (H256 hashes, u32 block numbers, MultiAddress) and works
/// for most solochains and parachains. Exposed at the crate root so the
/// mapping module can be concrete to it (see [`crate::ChainConfig`]).
pub type ChainConfig = PolkadotConfig;

/// A direct-RPC data source. Decodes each block against the metadata of its own
/// spec version, producing the framework's chain-agnostic [`Block`](palletide_core::Block) model.
pub type SubxtSource = RpcSource<ChainConfig>;

/// RPC source for a custom Subxt configuration. Its header implements [`ParentHashHeader`].
pub struct RpcSource<T: Config> {
    clients: Vec<tokio::sync::RwLock<Option<OnlineClient<T>>>>,
    urls: Vec<String>,
    chain: T,
    genesis: subxt::config::HashFor<T>,
    reconnect: Mutex<()>,
    active: AtomicUsize,
    config: SourceConfig,
    /// The live finalized-block stream, lazily created on first `next_finalized`.
    /// Wrapped so `next_finalized(&self)` can advance it without `&mut self`.
    finalized_stream: Mutex<Option<subxt::client::Blocks<T>>>,
}

/// Local header adapter; custom chain headers expose their parent hash here.
/// This avoids requiring an unreleased Subxt API.
pub trait ParentHashHeader: subxt::config::Header {
    /// Raw parent hash bytes.
    fn parent_hash_bytes(&self) -> &[u8];
}

impl<H: subxt::config::Hash> ParentHashHeader for subxt::config::substrate::SubstrateHeader<H>
where
    Self: subxt::config::Header,
{
    fn parent_hash_bytes(&self) -> &[u8] {
        self.parent_hash.as_ref()
    }
}

impl<T: Config> RpcSource<T>
where
    T::Header: ParentHashHeader,
{
    /// Connect using the chain's default configuration.
    pub async fn connect(config: SourceConfig) -> Result<Self>
    where
        T: Default,
    {
        Self::connect_with_config(config, T::default()).await
    }

    async fn open_endpoint(
        chain: T,
        url: &str,
    ) -> Result<(OnlineClient<T>, subxt::config::HashFor<T>)> {
        use subxt::backend::Backend;
        let rpc = subxt::rpcs::RpcClient::from_url(url)
            .await
            .map_err(|e| PalletideError::Source(format!("RPC connect: {e}")))?;
        let backend = subxt::backend::CombinedBackend::<T>::builder()
            .build_with_background_driver(rpc)
            .await
            .map_err(|e| PalletideError::Source(format!("RPC backend: {e}")))?;
        // Query the backend rather than a possibly configured genesis constant.
        let genesis = backend
            .genesis_hash()
            .await
            .map_err(|e| PalletideError::Source(format!("RPC genesis: {e}")))?;
        let client =
            OnlineClient::<T>::from_backend_with_config(chain, std::sync::Arc::new(backend))
                .await
                .map_err(|e| PalletideError::Source(format!("RPC client: {e}")))?;
        Ok((client, genesis))
    }

    /// Connect with an explicit Subxt configuration and bounded endpoint failover.
    pub async fn connect_with_config(config: SourceConfig, chain: T) -> Result<Self> {
        if config.ss58_prefix > 16383 || matches!(config.ss58_prefix, 46 | 47) {
            return Err(PalletideError::Source(
                "invalid or reserved SS58 prefix".into(),
            ));
        }
        let mut clients = Vec::new();
        let mut genesis = None;
        let mut active = None;
        let urls: Vec<String> = std::iter::once(config.url.clone())
            .chain(config.fallback_urls.iter().cloned())
            .collect();
        for url in &urls {
            match tokio::time::timeout(
                config.request_timeout,
                Self::open_endpoint(chain.clone(), url),
            )
            .await
            {
                Ok(Ok((client, actual_genesis))) => {
                    if genesis.is_some_and(|hash| hash != actual_genesis) {
                        return Err(PalletideError::Source(
                            "fallback endpoint belongs to a different chain".into(),
                        ));
                    }
                    genesis = Some(actual_genesis);
                    if active.is_none() {
                        active = Some(clients.len());
                    }
                    clients.push(tokio::sync::RwLock::new(Some(client)));
                }
                _ => {
                    tracing::warn!("RPC endpoint connection failed or timed out");
                    clients.push(tokio::sync::RwLock::new(None));
                }
            }
        }
        let Some(active) = active else {
            return Err(PalletideError::Source("no RPC endpoint available".into()));
        };
        Ok(Self {
            clients,
            active: AtomicUsize::new(active),
            urls,
            chain,
            genesis: genesis.expect("connected endpoint"),
            reconnect: Mutex::new(()),
            config,
            finalized_stream: Mutex::new(None),
        })
    }

    async fn client(&self) -> Result<OnlineClient<T>> {
        self.clients[self.active.load(Ordering::Relaxed) % self.clients.len()]
            .read()
            .await
            .clone()
            .ok_or_else(|| PalletideError::Source("RPC endpoint unavailable".into()))
    }

    async fn rotate_and_reconnect(&self) {
        let Some(_guard) = self.reconnect.try_lock() else {
            return;
        };
        let index =
            self.active.fetch_add(1, Ordering::Relaxed).wrapping_add(1) % self.clients.len();
        match tokio::time::timeout(
            self.config.request_timeout,
            Self::open_endpoint(self.chain.clone(), &self.urls[index]),
        )
        .await
        {
            Ok(Ok((client, genesis))) if genesis == self.genesis => {
                *self.clients[index].write().await = Some(client);
            }
            _ => tracing::warn!("RPC reconnection failed or returned a different chain"),
        }
    }

    async fn bounded<F, R>(&self, operation: F) -> Result<R>
    where
        F: std::future::Future<Output = Result<R>>,
    {
        let result = tokio::time::timeout(self.config.request_timeout, operation)
            .await
            .unwrap_or_else(|_| Err(PalletideError::Source("RPC request timed out".into())));
        if matches!(&result, Err(PalletideError::Source(_))) {
            self.rotate_and_reconnect().await;
        }
        result
    }

    /// Fetch and map a single block by height, retrying transient RPC failures
    /// per the configured [`RetryConfig`](crate::RetryConfig). `finalized` records
    /// whether the caller considers this height final.
    async fn fetch_one(
        &self,
        height: BlockNumber,
        finalized: bool,
    ) -> Result<palletide_core::Block> {
        retry_async(self.config.retry, "fetch_block", || {
            self.bounded(async move {
                let at = self
                    .client()
                    .await?
                    .at_block(height)
                    .await
                    .map_err(|e| PalletideError::Source(format!("at_block {height}: {e}")))?;
                map_block(
                    &at,
                    finalized,
                    self.config.selection,
                    self.config.ss58_prefix,
                    self.config.strict,
                )
                .await
            })
        })
        .await
    }

    /// One attempt at pulling the next finalized block: lazily subscribe the
    /// stream if needed, then take one block. Any error here is surfaced to the
    /// retry wrapper in `next_finalized`, which resets the subscription and
    /// retries. An ended stream is a retryable source failure.
    async fn next_finalized_once(&self) -> Result<BlockBatch> {
        let mut guard = self.finalized_stream.lock().await;
        if guard.is_none() {
            // `stream_blocks` yields finalized blocks (per subxt docs).
            let stream = self
                .client()
                .await?
                .stream_blocks()
                .await
                .map_err(|e| PalletideError::Source(format!("subscribe finalized: {e}")))?;
            *guard = Some(stream);
        }
        let stream = guard.as_mut().expect("just set");

        match stream.next().await {
            Some(Ok(block)) => {
                let at = block
                    .at()
                    .await
                    .map_err(|e| PalletideError::Source(format!("at finalized block: {e}")))?;
                let mapped = map_block(
                    &at,
                    true,
                    self.config.selection,
                    self.config.ss58_prefix,
                    self.config.strict,
                )
                .await?;
                Ok(BlockBatch {
                    blocks: vec![mapped],
                })
            }
            Some(Err(e)) => Err(PalletideError::Source(format!("finalized stream: {e}"))),
            None => Err(PalletideError::Source(
                "finalized subscription ended".into(),
            )),
        }
    }
}

#[async_trait]
impl<T: Config> DataSource for RpcSource<T>
where
    T::Header: ParentHashHeader,
{
    async fn finalized_head(&self) -> Result<BlockNumber> {
        retry_async(self.config.retry, "finalized_head", || {
            self.bounded(async {
                // `at_current_block` instantiates a client at the current *finalized*
                // block at the time of the call (per subxt docs).
                let at = self
                    .client()
                    .await?
                    .at_current_block()
                    .await
                    .map_err(|e| PalletideError::Source(format!("finalized head: {e}")))?;
                Ok(at.block_number() as u32)
            })
        })
        .await
    }

    async fn fetch_batch(&self, from: BlockNumber, to: BlockNumber) -> Result<BlockBatch> {
        if from > to {
            return Ok(BlockBatch { blocks: vec![] });
        }
        // Cap the returned span to the configured batch size.
        let end = to.min(from.saturating_add(self.config.batch_size.saturating_sub(1)));

        // Direct RPC is latency-bound, so fetch the span's blocks CONCURRENTLY
        // (up to `concurrency` in flight) rather than one-at-a-time. `buffered`
        // preserves input order, so the returned blocks stay in height order.
        // Anything in [from, to] requested as a batch is treated as finalized
        // (backfill range); live unfinalized blocks come via `next_finalized`.
        use futures::stream::{self, StreamExt, TryStreamExt};
        let blocks: Vec<palletide_core::Block> = stream::iter(from..=end)
            .map(|h| self.fetch_one(h, true))
            .buffered(self.config.concurrency)
            .try_collect()
            .await?;

        Ok(BlockBatch { blocks })
    }

    async fn next_finalized(&self) -> Result<BlockBatch> {
        // Retry transient failures. Crucially, on any error we drop the stored
        // subscription (`take()` below) so the next attempt re-subscribes — this
        // is the reconnect path when the finalized-block stream is dropped.
        retry_async(self.config.retry, "next_finalized", || async {
            match self.bounded(self.next_finalized_once()).await {
                Ok(batch) => Ok(batch),
                Err(e) => {
                    // Reset the subscription so the retry reconnects from scratch.
                    self.finalized_stream.lock().await.take();
                    Err(e)
                }
            }
        })
        .await
    }

    fn name(&self) -> &str {
        "subxt-rpc"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn custom_config_connection_failure_is_bounded() {
        let config = SourceConfig::new("ws://127.0.0.1:1")
            .with_fallback_urls(vec!["ws://127.0.0.1:2".into()])
            .with_request_timeout(std::time::Duration::from_millis(20));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            RpcSource::<subxt::config::SubstrateConfig>::connect(config),
        )
        .await;
        assert!(result.unwrap().is_err());
    }
}
