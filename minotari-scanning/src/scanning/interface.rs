use async_trait::async_trait;
use tari_node_components::blocks::Block;
use tari_transaction_components::transaction_components::TransactionOutput;

use crate::{BlockHeaderInfo, BlockScanResult, ScanConfig, TipInfo, WalletResult};

/// Blockchain scanner trait for scanning UTXOs
///
/// This trait provides a lightweight interface that can be implemented by
/// different backend providers (gRPC, HTTP, etc.) without requiring heavy
/// dependencies in the core library.
#[async_trait]
pub trait BlockchainScanner: Send + Sync {
    /// Scan for wallet outputs in the specified block range
    async fn scan_blocks(&mut self, config: &ScanConfig) -> WalletResult<(Vec<BlockScanResult>, bool)>;
    /// Get the current chain tip information
    async fn get_tip_info(&mut self) -> WalletResult<TipInfo>;

    /// Search for specific UTXOs by commitment
    async fn search_utxos(&mut self, commitments: Vec<Vec<u8>>) -> WalletResult<Vec<BlockScanResult>>;

    /// Fetch specific UTXOs by hash
    async fn fetch_utxos(&mut self, hashes: Vec<Vec<u8>>) -> WalletResult<Vec<TransactionOutput>>;

    /// Get blocks by height range
    async fn get_blocks_by_heights(&mut self, heights: Vec<u64>) -> WalletResult<Vec<Block>>;

    /// Get a single block by height
    async fn get_block_by_height(&mut self, height: u64) -> WalletResult<Option<Block>>;

    async fn get_header_by_height(&mut self, height: u64) -> WalletResult<Option<BlockHeaderInfo>>;
}
