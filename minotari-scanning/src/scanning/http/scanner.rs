//! HTTP-based blockchain scanner implementation
//!
//! This module provides an HTTP implementation of the `BlockchainScanner` trait
//! that connects to a Tari base node via HTTP API to scan for wallet outputs.

// Native targets use reqwest
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::{
    BlockHeaderInfo, UtxoScanResult,
    errors::{WalletError, WalletResult},
    http::models::{HttpBlockHeader, HttpTipInfoResponse, IncompleteScannedOutput, ScanningOutputStruct},
    scanning::{BlockScanResult, InProgressScan, ScanConfig, TipInfo, interface::BlockchainScanner},
};
use async_trait::async_trait;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use reqwest::Client;
use tari_common_types::types::FixedHash;
use tari_node_components::blocks::Block;
use tari_transaction_components::{
    key_manager::TransactionKeyManagerInterface,
    rpc::models::{BlockUtxoInfo, GetUtxosByBlockResponse, SyncUtxosByBlockResponseV0, SyncUtxosByBlockResponseV1},
    transaction_components::TransactionOutput,
};
use tari_utilities::hex::Hex;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::block_in_place;
use tracing::log::error;
use tracing::{debug, warn};
const SYNC_UTXOS_BY_BLOCK_PAGE_LIMIT: u64 = 50;
const HTTP2_INITIAL_WINDOW_SIZE: u32 = 4 * 1024 * 1024;
const HTTP2_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);
const HTTP2_KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP client for connecting to Tari base node
#[derive(Clone)]
pub struct HttpBlockchainScanner<KM> {
    /// HTTP client for making requests (native targets)
    client: Client,
    /// Base URL for the HTTP API
    base_url: String,
    /// Request timeout (native targets only)
    timeout: Duration,
    key_managers: Vec<KM>,
    current_in_progress: InProgressScan,
    thread_count: usize,
}

impl<KM> HttpBlockchainScanner<KM>
where
    KM: TransactionKeyManagerInterface,
{
    /// Create a new HTTP scanner with the given base URL
    pub async fn new(base_url: String, key_managers: Vec<KM>, number_processing_threads: usize) -> WalletResult<Self> {
        let timeout = Duration::from_secs(30);
        Self::with_timeout(base_url, timeout, key_managers, number_processing_threads).await
    }

    /// Create a new HTTP scanner with custom timeout (native only)
    pub async fn with_timeout(
        base_url: String,
        timeout: Duration,
        key_managers: Vec<KM>,
        number_processing_threads: usize,
    ) -> WalletResult<Self> {
        let thread_count = if number_processing_threads > 0 {
            number_processing_threads
        } else {
            (num_cpus::get().saturating_sub(2)).max(1)
        };
        if key_managers.is_empty() {
            return Err(WalletError::ConfigurationError(
                "At least one key manager must be specified".to_string(),
            ));
        }

        let client = Client::builder()
            .http2_initial_stream_window_size(HTTP2_INITIAL_WINDOW_SIZE)
            .http2_initial_connection_window_size(HTTP2_INITIAL_WINDOW_SIZE)
            .tcp_nodelay(true)
            .http2_keep_alive_interval(HTTP2_KEEP_ALIVE_INTERVAL)
            .http2_keep_alive_timeout(HTTP2_KEEP_ALIVE_TIMEOUT)
            .http2_keep_alive_while_idle(true)
            .timeout(timeout)
            .build()
            .map_err(|e| {
                WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                    "Failed to create HTTP client: {e}"
                )))
            })?;

        // Test the connection
        let test_url = format!("{base_url}/get_tip_info");
        let response = client.get(&test_url).send().await;
        if response.is_err() {
            let body = match response {
                Ok(resp) => resp.text().await.unwrap_or_default(),
                Err(e) => e.to_string(),
            };
            warn!("Connection test failed, response body: {body}");
            return Err(WalletError::ScanningError(
                crate::errors::ScanningError::blockchain_connection_failed(&format!("Failed to connect to {base_url}")),
            ));
        }

        Ok(Self {
            client,
            base_url,
            timeout,
            key_managers,
            current_in_progress: InProgressScan::new_empty(),
            thread_count,
        })
    }

    /// Sync UTXOs by block - matches WASM example usage
    async fn sync_utxos_by_block(
        &self,
        start_header_hash: &str,
        limit: u64,
        page: u64,
        exclude_spent: bool,
    ) -> WalletResult<SyncUtxosByBlockResponseV0> {
        let url = format!("{}/sync_utxos_by_block", self.base_url);
        let version = 1;
        let response = self
            .client
            .get(&url)
            .query(&[
                ("start_header_hash", start_header_hash),
                ("limit", &limit.to_string()),
                ("page", &page.to_string()),
                ("version", &version.to_string()),
                ("exclude_spent", &exclude_spent.to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                    "HTTP request failed: {e}"
                )))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("HTTP error response body: {}", body);
            return Err(WalletError::ScanningError(
                crate::errors::ScanningError::blockchain_connection_failed(&format!("HTTP error: {status}")),
            ));
        }

        let sync_response_v1: SyncUtxosByBlockResponseV1 = response.json().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to parse response: {e}"
            )))
        })?;

        let sync_response = sync_response_v1.into();

        Ok(sync_response)
    }

    async fn get_utxos_by_block(&self, current_header_hash: &str) -> WalletResult<GetUtxosByBlockResponse> {
        let url = format!("{}/get_utxos_by_block", self.base_url);

        let response = self
            .client
            .get(&url)
            .query(&[("header_hash", current_header_hash)])
            .send()
            .await
            .map_err(|e| {
                WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                    "HTTP request failed: {e}"
                )))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("HTTP error response body: {}", body);
            return Err(WalletError::ScanningError(
                crate::errors::ScanningError::blockchain_connection_failed(&format!("HTTP error: {status}")),
            ));
        }

        let body_text = response.text().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to read response body: {e}"
            )))
        })?;
        let sync_response: GetUtxosByBlockResponse = serde_json::from_str(&body_text).map_err(|e| {
            warn!("Failed to parse response body: {}", body_text);
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to parse response: {e}"
            )))
        })?;

        Ok(sync_response)
    }

    /// Create a scan config with wallet keys for block scanning
    pub const fn create_scan_config_with_wallet_keys(
        &self,
        start_height: u64,
        end_height: Option<u64>,
    ) -> WalletResult<ScanConfig> {
        Ok(ScanConfig {
            start_height,
            end_height,
            batch_size: Some(50),
            request_timeout: self.timeout,
            exclude_spent: false,
        })
    }

    /// Scan for regular recoverable outputs using encrypted data decryption
    fn scan_for_recoverable_output(
        &self,
        output: &ScanningOutputStruct,
    ) -> WalletResult<Option<IncompleteScannedOutput>> {
        for (index, key_manager) in self.key_managers.iter().enumerate() {
            if let Some((commitment_mask, value, memo)) = key_manager.try_output_key_recovery(
                &output.commitment,
                &output.encrypted_data,
                &output.sender_offset_public_key,
            )? {
                let output = IncompleteScannedOutput::new(output, value, commitment_mask, memo, index)?;
                return Ok(Some(output));
            }
        }
        Ok(None)
    }

    /// Fetch block range using the `sync_utxos_by_block` endpoint
    #[allow(clippy::cognitive_complexity)]
    async fn fetch_block_range(&mut self) -> WalletResult<(Vec<BlockUtxoInfo>, bool)> {
        let start_height = self.current_in_progress.get_config().map_or(0, |c| c.start_height);
        let exclude_spent = self.current_in_progress.get_config().is_some_and(|c| c.exclude_spent);

        // Get the starting header hash
        let mut more_blocks = true;
        let current_header_hash = if let Some(h) = self.current_in_progress.get_header() {
            h.clone()
        } else {
            let Some(start_header) = self.get_header_by_height(start_height).await? else {
                return Err(WalletError::ScanningError(
                    crate::errors::ScanningError::blockchain_connection_failed(&format!(
                        "Failed to get header at height {start_height}"
                    )),
                ));
            };
            let current_header_hash = start_header.hash.to_hex();
            self.current_in_progress.set_next_request(current_header_hash.clone());
            current_header_hash
        };

        let mut all_blocks = Vec::new();

        debug!("Starting fetch_block_range from height {} ", start_height);
        let limit = self
            .current_in_progress
            .get_config()
            .and_then(|c| c.batch_size)
            .unwrap_or(SYNC_UTXOS_BY_BLOCK_PAGE_LIMIT);
        let page = self.current_in_progress.page();
        let sync_response = self
            .sync_utxos_by_block(&current_header_hash, limit, page, exclude_spent)
            .await?;
        if sync_response.blocks.is_empty() {
            debug!("No more blocks available from base node");
            return Ok((Vec::new(), false));
        }
        let mut has_next_page = sync_response.has_next_page;
        let next_header_to_scan = sync_response.next_header_to_scan.clone();
        let blocks_to_process = sync_response.blocks.into_iter();

        // Add all blocks from this response
        for block in blocks_to_process {
            if let Some(end_height) = self.current_in_progress.get_config().and_then(|c| c.end_height)
                && block.height > end_height
            {
                debug!("Reached end height {}, stopping fetch", end_height);
                self.current_in_progress.clear();
                more_blocks = false;
                has_next_page = false;
            }
            all_blocks.push(block);
        }
        self.current_in_progress.increment_page();

        if !has_next_page && self.current_in_progress.is_active() {
            // we are done scanning this batch of blocks, we need to request the next header, and we have not
            // reached some end goal
            if next_header_to_scan.is_empty() {
                debug!("No next header to scan, ending fetch");
                more_blocks = false;
                self.current_in_progress.clear();
            } else {
                let next_header_to_scan_hex = next_header_to_scan.to_hex();
                debug!("Setting next header to scan: {}", next_header_to_scan_hex);
                // Safeguard against infinite loops if the server returns the same hash
                if next_header_to_scan_hex == self.current_in_progress.get_header().cloned().unwrap_or_default() {
                    debug!("Next header is the same as the current one, stopping to prevent infinite loop.");
                    more_blocks = false;
                    self.current_in_progress.clear();
                } else {
                    self.current_in_progress.set_next_request(next_header_to_scan_hex);
                }
            }
        }

        debug!("Fetched {} blocks for range {}", all_blocks.len(), start_height,);

        Ok((all_blocks, more_blocks))
    }

    pub fn update_scan_config(&mut self, config: &ScanConfig) -> WalletResult<()> {
        debug!(
            "String new scan, scanning from: {} to  {:?}",
            config.start_height, config.end_height
        );
        self.current_in_progress = InProgressScan::new(config.clone());
        Ok(())
    }

    pub fn clear_in_progress_scan(&mut self) {
        self.current_in_progress.clear();
    }
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl<KM> BlockchainScanner for HttpBlockchainScanner<KM>
where
    KM: TransactionKeyManagerInterface,
{
    async fn scan_blocks(
        &mut self,
        config: &ScanConfig,
    ) -> WalletResult<mpsc::Receiver<WalletResult<Vec<BlockScanResult>>>> {
        if let Some(end_height) = config.end_height
            && config.start_height > end_height
        {
            return Err(WalletError::OperationNotSupported(
                "start_height cannot be greater than end_height".to_string(),
            ));
        }

        let (send_scan_result, mut rec_scan_result) = mpsc::channel(1000);
        let (send_download, rec_download) = mpsc::channel(1000);
        self.update_scan_config(config)?;
        let mut download_scanner = self.clone();
        let send_download_1 = send_download.clone();
        let scan_config = config.clone();
        tokio::spawn(async move {
            let tip = match download_scanner.get_tip_info().await {
                Ok(tip) => tip,
                Err(e) => {
                    error!("Failed to get tip info: {}", e);
                    return;
                },
            };
            if scan_config.start_height > tip.best_block_height {
                debug!(
                    "Tip height {} is less than requested start height {}, returning empty results",
                    tip.best_block_height, scan_config.start_height
                );

                send_download_1
                    .send(Ok(Vec::<BlockScanResult>::new()))
                    .await
                    .inspect_err(|e| {
                        error!("Failed to send tip result with error: {}", e);
                    });
                return;
            }
            loop {
                let more_blocks = match download_scanner.fetch_block_range().await {
                    Ok((http_blocks, more_blocks)) => {
                        if let Err(e) = send_scan_result.send(Ok(http_blocks)).await {
                            error!("Failed to send download result with error: {}", e);
                            return;
                        };
                        more_blocks
                    },
                    Err(e) => {
                        send_download_1.send(Err(e)).await.inspect_err(|e| {
                            error!("Failed to send download result with error: {}", e);
                        });
                        return;
                    },
                };

                if !more_blocks {
                    break;
                }
            }
            debug!("Finished downloading blocks");
            if let Err(e) = send_download_1.send(Ok(Vec::new())).await {
                error!("Failed to send download result with error: {}", e);
                return;
            };
        });
        let thread_count = self.thread_count;
        let mut processing_scanner = self.clone();
        tokio::spawn(async move {
            let processing_pool = match rayon::ThreadPoolBuilder::new()
                .num_threads(thread_count)
                .build()
                .map_err(|e| WalletError::ConfigurationError(format!("Failed to build thread pool: {e}")))
            {
                Ok(pool) => pool,
                Err(e) => {
                    send_download.send(Err(e)).await.inspect_err(|e| {
                        error!("Failed to send download result with error: {}", e);
                    });
                    return;
                },
            };
            while let Some(http_blocks_res) = rec_scan_result.recv().await {
                let errors = RwLock::new(Vec::new());
                let results = RwLock::new(Vec::new());
                let http_blocks = match http_blocks_res {
                    Ok(http_blocks) => http_blocks,
                    Err(e) => {
                        send_download.send(Err(e)).await.inspect_err(|e| {
                            error!("Failed to send download error with error: {}", e);
                        });
                        return;
                    },
                };
                let handle = Handle::current();
                block_in_place(|| {
                    processing_pool.install(|| {
                        http_blocks.into_par_iter().for_each(|http_block| {
                            let mut wallet_outputs = Vec::new();

                            let header_hash = match FixedHash::try_from(http_block.header_hash.clone()) {
                                Ok(h) => h,
                                Err(e) => {
                                    errors
                                        .write()
                                        .expect("write lock should not be poisoned")
                                        .push(WalletError::ConversionError(e.to_string()));
                                    return;
                                },
                            };
                            for output in &http_block.outputs {
                                let scanned_output = match output.clone().try_into() {
                                    Ok(o) => o,
                                    Err(e) => {
                                        errors.write().expect("write lock should not be poisoned").push(e);
                                        continue;
                                    },
                                };
                                match processing_scanner.scan_for_recoverable_output(&scanned_output) {
                                    Ok(Some(wallet_output)) => {
                                        wallet_outputs.push(wallet_output);
                                    },
                                    Ok(None) => {},
                                    Err(e) => {
                                        errors.write().expect("write lock should not be poisoned").push(e);
                                    },
                                }
                            }
                            let mut procssed_outputs = Vec::new();
                            if !wallet_outputs.is_empty() {
                                let block_response = match handle
                                    .block_on(processing_scanner.get_utxos_by_block(&header_hash.to_hex()))
                                {
                                    Ok(r) => r,
                                    Err(e) => {
                                        errors.write().expect("write lock should not be poisoned").push(e);
                                        return;
                                    },
                                };
                                for output in &wallet_outputs {
                                    if let Some(index) = block_response
                                        .outputs
                                        .iter()
                                        .position(|o| *o.encrypted_data() == output.encrypted_data)
                                    {
                                        let tx_output =
                                            block_response.outputs.get(index).expect("should exist").clone();
                                        let output_hash = output.output_hash;
                                        // Attempt to convert to wallet output
                                        match output.to_wallet_output(
                                            tx_output,
                                            processing_scanner
                                                .key_managers
                                                .get(output.key_manager_index)
                                                .expect("should exist"),
                                        ) {
                                            Ok(Some(wallet_output)) => {
                                                procssed_outputs.push((
                                                    output_hash,
                                                    wallet_output,
                                                    output.key_manager_index,
                                                ));
                                            },
                                            Ok(None) => {},
                                            Err(e) => {
                                                errors.write().expect("Write lock should not be poisoned").push(e);
                                            },
                                        }
                                    }
                                }
                            }

                            results
                                .write()
                                .expect("lock should not be poisoned")
                                .push(BlockScanResult {
                                    height: http_block.height,
                                    block_hash: header_hash,
                                    wallet_outputs: procssed_outputs,
                                    inputs: http_block
                                        .inputs
                                        .into_iter()
                                        .map(|i| FixedHash::try_from(i).unwrap_or_default())
                                        .collect(),
                                    mined_timestamp: http_block.mined_timestamp,
                                });
                        });
                    });
                });
                let first_error = errors.read().expect("...").first().cloned();
                if let Some(e) = first_error {
                    if let Err(err) = send_download.send(Err(e)).await {
                        error!("Failed to send download error with error: {}", err);
                    };
                    return;
                }
                let mut results = results.into_inner().expect("Lock should not be poisoned");
                results.sort_by_key(|a| a.height);

                debug!(
                    "HTTP batch completed, found {} blocks with wallet outputs",
                    results.len(),
                );
                send_download.send(Ok(results)).await.inspect_err(|e| {
                    error!("Failed to send download error with error: {}", e);
                });
            }
            debug!("HTTP scan completed, found",);
        });

        Ok(rec_download)
    }

    async fn get_tip_info(&mut self) -> WalletResult<TipInfo> {
        let url = format!("{}/get_tip_info", self.base_url);

        // Native implementation using reqwest
        let response = self.client.get(&url).send().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "HTTP request failed: {e}"
            )))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("HTTP error response body: {}", body);
            return Err(WalletError::ScanningError(
                crate::errors::ScanningError::blockchain_connection_failed(&format!("HTTP error: {status}")),
            ));
        }

        let body_text = response.text().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to read response body: {e}"
            )))
        })?;

        let tip_response: HttpTipInfoResponse = serde_json::from_str(&body_text).map_err(|e| {
            warn!("Failed to parse response body: {}", body_text);
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to parse response: {e}"
            )))
        })?;

        Ok(TipInfo {
            best_block_height: tip_response.metadata.best_block_height,
            best_block_hash: FixedHash::try_from(tip_response.metadata.best_block_hash)
                .map_err(|e| WalletError::ConversionError(e.to_string()))?,
            accumulated_difficulty: tip_response.metadata.accumulated_difficulty,
            pruned_height: tip_response.metadata.pruned_height,
            timestamp: tip_response.metadata.timestamp,
        })
    }

    async fn search_utxos(&mut self, _commitments: Vec<Vec<u8>>) -> WalletResult<Vec<BlockScanResult>> {
        // This endpoint is not implemented in the current HTTP API
        // It would require a different endpoint that searches for specific commitments
        Err(WalletError::ScanningError(
            crate::errors::ScanningError::blockchain_connection_failed("search_utxos not implemented for HTTP scanner"),
        ))
    }

    async fn fetch_utxos(&mut self, _hashes: Vec<Vec<u8>>) -> WalletResult<Vec<TransactionOutput>> {
        // This endpoint is not implemented in the current HTTP API
        // It would require a different endpoint that fetches specific UTXOs by hash
        Err(WalletError::ScanningError(
            crate::errors::ScanningError::blockchain_connection_failed("fetch_utxos not implemented for HTTP scanner"),
        ))
    }

    async fn get_blocks_by_heights(&mut self, heights: Vec<u64>) -> WalletResult<Vec<Block>> {
        let mut blocks = Vec::new();

        for height in heights {
            if let Some(block) = self.get_block_by_height(height).await? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }

    async fn get_block_by_height(&mut self, _height: u64) -> WalletResult<Option<Block>> {
        // method does not exit
        Ok(None)
    }

    async fn get_header_by_height(&mut self, height: u64) -> WalletResult<Option<BlockHeaderInfo>> {
        use tari_utilities::epoch_time::EpochTime;

        let url = format!("{}/get_header_by_height?height={}", self.base_url, height);

        let response = self.client.get(&url).send().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "HTTP request failed: {e}"
            )))
        })?;

        if !response.status().is_success() {
            if response.status() == 404 {
                return Ok(None);
            }
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!("HTTP error response body: {}", body);
            return Err(WalletError::ScanningError(
                crate::errors::ScanningError::blockchain_connection_failed(&format!("HTTP error: {status}")),
            ));
        }

        let body = response.text().await.map_err(|e| {
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to read response body: {e}"
            )))
        })?;

        let header_response: HttpBlockHeader = serde_json::from_str(&body).map_err(|e| {
            warn!("Failed to parse response body: {}", body);
            WalletError::ScanningError(crate::errors::ScanningError::blockchain_connection_failed(&format!(
                "Failed to parse response: {e}"
            )))
        })?;

        Ok(Some(BlockHeaderInfo {
            height: header_response.height,
            hash: FixedHash::try_from(header_response.hash).map_err(|e| WalletError::ConversionError(e.to_string()))?,
            timestamp: EpochTime::from(header_response.timestamp),
        }))
    }
}
