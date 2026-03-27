//! High-level HTTP client for wallet-to-base-node communication.
//!
//! This module provides the [`WalletHttpClient`] struct, which is the primary
//! interface for wallets to interact with Tari base nodes via HTTP/JSON-RPC.
//!
//! # Overview
//!
//! The `WalletHttpClient` wraps the low-level HTTP client and provides
//! domain-specific methods for common wallet operations:
//!
//! - Submitting transactions to the network
//! - Querying transaction status by signature
//! - Querying blockchain tip and fee statistics
//!
//! # Example
//!
//! ```rust,no_run
//! use url::Url;
//! use minotari::http::WalletHttpClient;
//!
//! # async fn example() -> Result<(), anyhow::Error> {
//! let client = WalletHttpClient::new(Url::parse("http://localhost:18142")?)?;
//!
//! let tip = client.get_tip_info().await?;
//! println!("Chain height: {:?}", tip.metadata.map(|m| m.best_block_height()));
//! # Ok(())
//! # }
//! ```

use log::{debug, info, warn};
use reqwest::Method;
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::rpc::models::{
    FeePerGramStat, TipInfoResponse, TxQueryResponse, TxSubmissionResponse,
};
use tari_transaction_components::transaction_components::Transaction;
use tari_utilities::hex::to_hex;
use url::Url;

use crate::http::utils::check_transaction_size;
use crate::http::{GetMempoolFeePerGramStatsResponse, JsonRpcResponse};

use super::http_client::HttpClient;

/// HTTP client for wallet operations against a Tari base node.
///
/// This client provides high-level methods for common wallet operations,
/// handling the details of HTTP communication, JSON-RPC protocol, and
/// error handling internally.
///
/// # Features
///
/// - **Automatic Retries**: Transient failures are automatically retried
///   with exponential backoff
/// - **Latency Tracking**: Request latencies are tracked for monitoring
/// - **Size Validation**: Transactions are validated against RPC size limits
///   before submission
///
/// # Thread Safety
///
/// `WalletHttpClient` is safe to share across threads and can be used
/// from multiple async tasks concurrently.
///
/// # Example
///
/// ```rust,no_run
/// use url::Url;
/// use minotari::http::WalletHttpClient;
///
/// # async fn example() -> Result<(), anyhow::Error> {
/// let client = WalletHttpClient::new(Url::parse("http://localhost:18142")?)?;
///
/// let tip_info = client.get_tip_info().await?;
/// if tip_info.is_synced {
///     println!("Node is ready for transactions");
/// }
/// # Ok(())
/// # }
/// ```
pub struct WalletHttpClient {
    /// The underlying HTTP client that handles request/response processing.
    http_client: HttpClient,
}

impl WalletHttpClient {
    /// Creates a new wallet HTTP client with default configuration.
    ///
    /// Uses default settings of 30 second timeout and 3 retry attempts.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the Tari base node HTTP RPC endpoint
    ///   (e.g., `http://localhost:18142`)
    ///
    /// # Returns
    ///
    /// Returns a configured `WalletHttpClient` instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized (e.g.,
    /// TLS backend initialization failure).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use url::Url;
    /// use minotari::http::WalletHttpClient;
    ///
    /// let client = WalletHttpClient::new(
    ///     Url::parse("http://localhost:18142").unwrap()
    /// ).unwrap();
    /// ```
    pub fn new(base_url: Url) -> Result<Self, anyhow::Error> {
        let http_client = HttpClient::new(base_url)?;
        Ok(Self { http_client })
    }

    /// Retrieves the current blockchain tip information from the base node.
    ///
    /// This method queries the `/get_tip_info` endpoint to get information
    /// about the current state of the blockchain, including block height
    /// and synchronization status.
    ///
    /// # Returns
    ///
    /// Returns [`TipInfoResponse`] containing the chain metadata and sync status.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The node is unreachable
    /// - The request times out
    /// - The response cannot be parsed
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use url::Url;
    /// use minotari::http::WalletHttpClient;
    ///
    /// # async fn example() -> Result<(), anyhow::Error> {
    /// let client = WalletHttpClient::new(Url::parse("http://localhost:18142")?)?;
    ///
    /// let tip = client.get_tip_info().await?;
    /// if let Some(metadata) = tip.metadata {
    ///     println!("Current height: {}", metadata.best_block_height());
    ///     println!("Accumulated difficulty: {}", metadata.accumulated_difficulty());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_tip_info(&self) -> Result<TipInfoResponse, anyhow::Error> {
        debug!("HTTP: Requesting tip info from base node");
        let response = self
            .http_client
            .send_request(Method::GET, "/get_tip_info", None)
            .await?;
        Ok(response)
    }

    /// Submits a transaction to the network via the base node.
    ///
    /// This method validates the transaction size, constructs a JSON-RPC
    /// request, and submits it to the base node's `/json_rpc` endpoint.
    ///
    /// # Arguments
    ///
    /// * `transaction` - The transaction to submit
    ///
    /// # Returns
    ///
    /// Returns [`TxSubmissionResponse`] indicating whether the transaction
    /// was accepted and the node's sync status.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transaction exceeds the maximum size limit (~1.99 MB)
    /// - The node is unreachable
    /// - The JSON-RPC request fails
    /// - The node rejects the transaction (error message included)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use url::Url;
    /// use minotari::http::WalletHttpClient;
    /// use tari_transaction_components::transaction_components::Transaction;
    ///
    /// # async fn example(transaction: Transaction) -> Result<(), anyhow::Error> {
    /// let client = WalletHttpClient::new(Url::parse("http://localhost:18142")?)?;
    ///
    /// let result = client.submit_transaction(transaction).await?;
    /// if result.accepted {
    ///     println!("Transaction accepted!");
    /// } else {
    ///     println!("Transaction rejected: {}", result.rejection_reason);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Transaction Validation
    ///
    /// Before submission, the transaction is serialized and its size is
    /// checked against the RPC frame limit. This prevents submission of
    /// transactions that would fail due to size constraints.
    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<TxSubmissionResponse, anyhow::Error> {
        info!(target: "audit", "HTTP: Submitting transaction");

        check_transaction_size(&transaction).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "submit_transaction",
            "params": { "transaction": transaction }
        });

        let response: JsonRpcResponse<TxSubmissionResponse> = self
            .http_client
            .send_request(Method::POST, "/json_rpc", Some(request))
            .await?;

        match response.result {
            Some(result) => {
                info!(target: "audit", "HTTP: Transaction submitted successfully");
                Ok(result)
            },
            None => {
                let error_msg = response.error.unwrap_or_else(|| "Unknown error".to_string());
                warn!(
                    target: "audit",
                    reason = &*error_msg;
                    "HTTP: Transaction submission failed"
                );
                Err(anyhow::anyhow!("Transaction submission failed: {}", error_msg))
            },
        }
    }

    /// Queries the status of a transaction by its excess signature.
    ///
    /// This method queries the `/transactions` endpoint to check whether
    /// a transaction has been mined, is in the mempool, or is unknown.
    ///
    /// # Arguments
    ///
    /// * `excess_sig_nonce` - The public nonce component of the excess signature
    /// * `excess_sig` - The signature component of the excess signature
    ///
    /// # Returns
    ///
    /// Returns [`TxQueryResponse`] containing the transaction's location
    /// and mining details if applicable.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The node is unreachable
    /// - The request times out
    /// - The response cannot be parsed
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use url::Url;
    /// use minotari::http::{WalletHttpClient};
    /// use tari_transaction_components::rpc::models::TxLocation;
    /// # async fn example() -> Result<(), anyhow::Error> {
    /// let client = WalletHttpClient::new(Url::parse("http://localhost:18142")?)?;
    ///
    /// let nonce = vec![0u8; 32];  // Example nonce bytes
    /// let sig = vec![0u8; 64];    // Example signature bytes
    ///
    /// let result = client.transaction_query(&nonce, &sig).await?;
    /// match result.location {
    ///     TxLocation::Mined => {
    ///         println!("Transaction mined at height {:?}", result.mined_height);
    ///     }
    ///     TxLocation::InMempool => {
    ///         println!("Transaction is pending in mempool");
    ///     }
    ///     TxLocation::None => {
    ///         println!("Transaction not found");
    ///     }
    ///     _ => {}
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Query Parameters
    ///
    /// The signature components are hex-encoded and passed as query
    /// parameters to the `/transactions` endpoint.
    pub async fn transaction_query(
        &self,
        excess_sig_nonce: &[u8],
        excess_sig: &[u8],
    ) -> Result<TxQueryResponse, anyhow::Error> {
        let excess_sig_nonce_hex = to_hex(excess_sig_nonce);
        let excess_sig_hex = to_hex(excess_sig);
        debug!(
            nonce = &*excess_sig_nonce_hex,
            sig = &*excess_sig_hex;
            "HTTP: Querying transaction"
        );

        let path = format!(
            "/transactions?excess_sig_nonce={}&excess_sig_sig={}",
            excess_sig_nonce_hex, excess_sig_hex
        );

        let response = self.http_client.send_request(Method::GET, &path, None).await?;

        debug!("HTTP: Transaction query successful");
        Ok(response)
    }

    pub async fn get_height_at_time(&self, epoch_time: u64) -> Result<u64, anyhow::Error> {
        debug!(
            time = epoch_time;
            "HTTP: Requesting block height at time"
        );
        let path = format!("/get_height_at_time?time={}", epoch_time);

        let response = self.http_client.send_request::<u64>(Method::GET, &path, None).await?;

        debug!("HTTP: Requesting block height successful");
        Ok(response)
    }

    /// Fetches the kernel merkle proof for a burn transaction kernel.
    ///
    /// Called after a burn transaction reaches sufficient confirmations to produce
    /// the [`EncodedMerkleProof`] needed for the complete burn claim proof.
    pub async fn get_kernel_merkle_proof(
        &self,
        excess_sig_nonce: &[u8],
        excess_sig: &[u8],
    ) -> Result<tari_transaction_components::rpc::models::GenerateKernelMerkleProofResponse, anyhow::Error> {
        let nonce_hex = to_hex(excess_sig_nonce);
        let sig_hex = to_hex(excess_sig);
        let path = format!(
            "/generate_kernel_merkle_proof?excess_sig_public_nonce={}&excess_sig_signature={}",
            nonce_hex, sig_hex,
        );
        let response = self.http_client.send_request(Method::GET, &path, None).await?;
        Ok(response)
    }

    pub async fn get_mempool_fee_per_gram_stats(&self, count: u64) -> Result<Vec<FeePerGramStat>, anyhow::Error> {
        debug!(
            count = count;
            "Requesting mempool fee per gram stats"
        );

        let path = format!("/get_mempool_fee_per_gram_stats?count={count}");

        let response = self
            .http_client
            .send_request::<GetMempoolFeePerGramStatsResponse>(Method::GET, &path, None)
            .await?;

        let stats: Vec<FeePerGramStat> = response
            .stats
            .into_iter()
            .map(|s| FeePerGramStat {
                order: s.order,
                min_fee_per_gram: MicroMinotari::from(s.min_fee_per_gram),
                avg_fee_per_gram: MicroMinotari::from(s.avg_fee_per_gram),
                max_fee_per_gram: MicroMinotari::from(s.max_fee_per_gram),
            })
            .collect();

        Ok(stats)
    }
}
