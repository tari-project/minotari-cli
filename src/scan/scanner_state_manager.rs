use crate::{ScanError, scan::coordinator::AccountSyncTarget};
use lightweight_wallet_libs::{HttpBlockchainScanner, ScanConfig};
use tari_transaction_components::key_manager::KeyManager;

pub struct ScannerStateManager {
    scanner: Option<HttpBlockchainScanner<KeyManager>>,
    active_indices: Vec<usize>,
    scan_config: ScanConfig,
}

impl ScannerStateManager {
    pub fn new() -> Self {
        Self {
            scanner: None,
            active_indices: Vec::new(),
            scan_config: ScanConfig::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_scanner_and_config(
        &mut self,
        new_active_indices: &[usize],
        new_start_height: u64,
        new_end_height: Option<u64>,
        batch_size: u64,
        all_targets: &[AccountSyncTarget],
        base_url: &str,
        processing_threads: usize,
    ) -> Result<(&mut HttpBlockchainScanner<KeyManager>, ScanConfig), ScanError> {
        // Only recreate scanner if accounts change
        if self.scanner.is_none() || self.active_indices != new_active_indices {
            let active_key_managers: Vec<KeyManager> = new_active_indices
                .iter()
                .map(|&idx| all_targets[idx].key_manager.clone())
                .collect();

            let new_scanner = HttpBlockchainScanner::new(base_url.to_string(), active_key_managers, processing_threads)
                .await
                .map_err(|e| ScanError::Intermittent(e.to_string()))?;

            self.scanner = Some(new_scanner);
            self.active_indices = new_active_indices.to_vec();
        }

        // Always update the config to the last known height.
        // This ensures the scanner always starts at a valid, existing block.
        self.scan_config = ScanConfig::default()
            .with_start_height(new_start_height)
            .with_batch_size(batch_size);
        self.scan_config.end_height = new_end_height;

        Ok((self.scanner.as_mut().unwrap(), self.scan_config.clone()))
    }
}
