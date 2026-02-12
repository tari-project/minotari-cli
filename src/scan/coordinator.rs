use lightweight_wallet_libs::{BlockScanResult, HttpBlockchainScanner, ScanConfig, scanning::BlockchainScanner};
use log::{info, warn};
use tari_common_types::{seeds::cipher_seed::BIRTHDAY_GENESIS_FROM_UNIX_EPOCH, types::PrivateKey};
use tari_transaction_components::key_manager::KeyManager;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::{
    PauseReason, ScanStatusEvent,
    db::{AccountRow, SqlitePool},
    http::WalletHttpClient,
    models::WalletEvent,
    scan::{
        DisplayedTransactionsEvent, ReorgDetectedEvent, ScanError, ScanMode, ScanRetryConfig, TransactionsUpdatedEvent,
        config::{MAX_BACKOFF_EXPONENT, MAX_BACKOFF_SECONDS, OPTIMAL_SCANNING_TREADS},
        events::{EventSender, ProcessingEvent},
        reorg,
        scan_db_handler::ScanDbHandler,
        scanner_state_manager::ScannerStateManager,
    },
    transactions::{MonitoringResult, MonitoringState, TransactionMonitor},
    webhooks::WebhookTriggerConfig,
};
use tari_transaction_components::key_manager::TransactionKeyManagerInterface;

/// Represents an account that is part of the current global scan session.
pub struct AccountSyncTarget {
    pub account: AccountRow,
    pub key_manager: KeyManager,
    pub view_key: PrivateKey,
    pub resume_height: u64,
    pub transaction_monitor: TransactionMonitor,
}

pub struct ScanCoordinator<E: EventSender> {
    pool: SqlitePool,
    base_url: String,
    client: WalletHttpClient,
    event_sender: E,
    retry_config: ScanRetryConfig,
    required_confirmations: u64,
    webhook_config: Option<WebhookTriggerConfig>,
    processing_threads: usize,
    reorg_check_interval: u64,
    batch_size: u64,
}

impl<E: EventSender + Clone + Send + 'static> ScanCoordinator<E> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        base_url: String,
        event_sender: E,
        retry_config: ScanRetryConfig,
        required_confirmations: u64,
        webhook_config: Option<WebhookTriggerConfig>,
        processing_threads: usize,
        reorg_check_interval: u64,
        batch_size: u64,
    ) -> Result<Self, ScanError> {
        let client = WalletHttpClient::new(
            base_url
                .parse()
                .map_err(|e| ScanError::Fatal(anyhow::anyhow!("Invalid Base URL: {}", e)))?,
        )
        .map_err(ScanError::Fatal)?;
        Ok(Self {
            pool,
            base_url,
            client,
            event_sender,
            retry_config,
            required_confirmations,
            webhook_config,
            processing_threads,
            reorg_check_interval,
            batch_size,
        })
    }

    /// The main entry point for the multi-account scan.
    pub async fn run(
        &self,
        accounts: Vec<AccountRow>,
        password: &str,
        mode: ScanMode,
        scanning_offset: u64,
        cancel_token: Option<CancellationToken>,
    ) -> Result<(Vec<WalletEvent>, bool), ScanError> {
        let mut conn = self.pool.get().map_err(|e| ScanError::DbError(e.into()))?;

        // Check reorgs for every account and find resume heights
        let mut sync_targets = Vec::new();
        for account in accounts {
            let target = self
                .prepare_target(account, password, &self.client, scanning_offset, &mut conn)
                .await?;
            sync_targets.push(target);
        }

        if sync_targets.is_empty() {
            return Ok((Vec::new(), false));
        }

        self.unified_scan_loop(sync_targets, mode, cancel_token).await
    }

    /// Prepares a scan context for an account.
    async fn prepare_target(
        &self,
        account: AccountRow,
        password: &str,
        wallet_client: &WalletHttpClient,
        scanning_offset: u64,
        conn: &mut rusqlite::Connection,
    ) -> Result<AccountSyncTarget, ScanError> {
        let key_manager = account.get_key_manager(password)?;
        let view_key = key_manager.get_private_view_key();

        // Check reorgs for this specific account before starting
        let mut scanner = HttpBlockchainScanner::new(
            self.base_url.clone(),
            vec![key_manager.clone()],
            OPTIMAL_SCANNING_TREADS,
        )
        .await
        .map_err(|e| ScanError::Intermittent(e.to_string()))?;

        let reorg_result = reorg::handle_reorgs(&mut scanner, conn, account.id, self.webhook_config.clone())
            .await
            .map_err(ScanError::Fatal)?;

        let mut resume_height = reorg_result.resume_height;

        if resume_height == 0 {
            // Use birthday logic
            let timestamp = (account.birthday as u64).saturating_sub(scanning_offset) * 24 * 60 * 60
                + BIRTHDAY_GENESIS_FROM_UNIX_EPOCH;
            resume_height = wallet_client
                .get_height_at_time(timestamp)
                .await
                .map_err(ScanError::Fatal)?;
        }
        resume_height = resume_height.saturating_sub(1);

        let monitor_state = MonitoringState::new();
        monitor_state.initialize(conn, account.id).map_err(ScanError::Fatal)?;

        let transaction_monitor =
            TransactionMonitor::new(monitor_state, self.required_confirmations, self.webhook_config.clone());

        Ok(AccountSyncTarget {
            account,
            key_manager,
            view_key,
            resume_height,
            transaction_monitor,
        })
    }

    async fn unified_scan_loop(
        &self,
        mut targets: Vec<AccountSyncTarget>,
        mode: ScanMode,
        cancel_token: Option<CancellationToken>,
    ) -> Result<(Vec<WalletEvent>, bool), ScanError> {
        let mut all_events = Vec::new();
        let db_handler = ScanDbHandler::new(self.pool.clone(), self.required_confirmations);
        let mut total_scanned_globally: u64 = 0;
        let mut blocks_since_reorg_check: u64 = 0;

        let mut state_manager = ScannerStateManager::new();

        loop {
            if let Some(token) = &cancel_token
                && token.is_cancelled()
            {
                return self.handle_pause(&targets, PauseReason::Cancelled, all_events);
            }

            let global_current_height = targets.iter().map(|t| t.resume_height).min().unwrap_or(0);

            // Determine which accounts are active at this height
            let active_indices: Vec<usize> = targets
                .iter()
                .enumerate()
                .filter(|(_, target)| target.resume_height <= global_current_height + self.batch_size)
                .map(|(idx, _)| idx)
                .collect();

            let next_horizon_height = targets
                .iter()
                .map(|t| t.resume_height)
                .filter(|&h| h > global_current_height + self.batch_size)
                .min();
            let end_height = next_horizon_height.map(|h| h.saturating_sub(1));

            let (scanner, scanner_config) = state_manager
                .get_scanner_and_config(
                    &active_indices,
                    global_current_height,
                    end_height,
                    self.batch_size,
                    &targets,
                    &self.base_url,
                    self.processing_threads,
                )
                .await?;
            let (scanned_blocks, more_blocks) = self.scan_blocks_with_timeout(scanner, &scanner_config).await?;

            if more_blocks {
                for &idx in &active_indices {
                    let target = &targets[idx];
                    self.event_sender
                        .send(ProcessingEvent::ScanStatus(ScanStatusEvent::MoreBlocksAvailable {
                            account_id: target.account.id,
                            last_scanned_height: global_current_height,
                        }));
                }
            }

            let mut max_new_height_in_batch = global_current_height;

            if !scanned_blocks.is_empty() {
                for &idx in &active_indices {
                    let target = &mut targets[idx];

                    // Get blocks that this account hasn't scanned yet
                    let blocks_for_account: Vec<BlockScanResult> = scanned_blocks
                        .iter()
                        .filter(|b| b.height > target.resume_height)
                        .cloned()
                        .map(|mut b| {
                            let relative_idx = active_indices.iter().position(|&i| i == idx).unwrap();
                            b.wallet_outputs.retain(|(_, _, scan_idx)| *scan_idx == relative_idx);
                            b
                        })
                        .collect();

                    if !blocks_for_account.is_empty() {
                        let events = db_handler
                            .process_blocks(
                                blocks_for_account.clone(),
                                target.account.id,
                                target.view_key.clone(),
                                self.event_sender.clone(),
                                target.transaction_monitor.has_pending_outbound(),
                                self.webhook_config.clone(),
                            )
                            .await?;
                        all_events.extend(events);

                        if let Some(last) = blocks_for_account.last() {
                            target.resume_height = last.height;
                            if last.height > max_new_height_in_batch {
                                max_new_height_in_batch = last.height;
                            }

                            let monitor_res = target
                                .transaction_monitor
                                .monitor_if_needed(&self.client, &self.pool, target.account.id, last.height)
                                .await
                                .map_err(ScanError::Fatal)?;

                            all_events.extend(monitor_res.wallet_events.clone());
                            self.emit_monitor_events(target.account.id, last.height, monitor_res);

                            db_handler.prune_tips(target.account.id, last.height).await?;

                            self.event_sender
                                .send(ProcessingEvent::ScanStatus(ScanStatusEvent::Progress {
                                    account_id: target.account.id,
                                    current_height: last.height,
                                    blocks_scanned: total_scanned_globally,
                                }));
                        }
                    }
                }
            }

            let new_blocks_count = max_new_height_in_batch.saturating_sub(global_current_height);
            blocks_since_reorg_check += new_blocks_count;
            total_scanned_globally += new_blocks_count;

            if blocks_since_reorg_check >= self.reorg_check_interval {
                self.check_global_reorgs(&mut targets, &db_handler).await?;
                blocks_since_reorg_check = 0;
            }

            // Handle Partial scan limits
            if let ScanMode::Partial { max_blocks } = mode
                && total_scanned_globally >= max_blocks
            {
                return self.handle_pause(
                    &targets,
                    PauseReason::MaxBlocksReached { limit: max_blocks },
                    all_events,
                );
            }

            if scanned_blocks.is_empty() || !more_blocks {
                for &idx in &active_indices {
                    let target = &targets[idx];
                    self.event_sender
                        .send(ProcessingEvent::ScanStatus(ScanStatusEvent::Completed {
                            account_id: target.account.id,
                            final_height: target.resume_height,
                            total_blocks_scanned: total_scanned_globally,
                        }));
                }

                if let ScanMode::Continuous { poll_interval } = mode {
                    self.wait_for_next_poll_cycle(&mut targets, &db_handler, poll_interval, &cancel_token)
                        .await?;
                    continue;
                }

                return Ok((all_events, false));
            }
        }
    }

    async fn check_global_reorgs(
        &self,
        targets: &mut [AccountSyncTarget],
        db_handler: &ScanDbHandler,
    ) -> Result<(), ScanError> {
        let mut conn = db_handler.get_connection().await?;
        for target in targets {
            let mut scanner = HttpBlockchainScanner::new(
                self.base_url.clone(),
                vec![target.key_manager.clone()],
                OPTIMAL_SCANNING_TREADS,
            )
            .await
            .map_err(|e| ScanError::Intermittent(e.to_string()))?;

            let res = reorg::handle_reorgs(&mut scanner, &mut conn, target.account.id, self.webhook_config.clone())
                .await
                .map_err(ScanError::Fatal)?;

            if let Some(info) = res.reorg_information {
                self.emit_reorg_event(target.account.id, info, res.resume_height);
                target.resume_height = res.resume_height.saturating_sub(1);
            }
        }
        Ok(())
    }

    async fn wait_for_next_poll_cycle(
        &self,
        targets: &mut [AccountSyncTarget],
        db_handler: &ScanDbHandler,
        interval: std::time::Duration,
        cancel: &Option<CancellationToken>,
    ) -> Result<(), ScanError> {
        for target in targets.iter() {
            self.event_sender
                .send(ProcessingEvent::ScanStatus(ScanStatusEvent::Waiting {
                    account_id: target.account.id,
                    resume_in: interval,
                }));
        }

        if let Some(token) = cancel {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {},
                _ = token.cancelled() => return Ok(()),
            }
        } else {
            tokio::time::sleep(interval).await;
        }

        self.check_global_reorgs(targets, db_handler).await
    }

    fn handle_pause(
        &self,
        targets: &[AccountSyncTarget],
        reason: PauseReason,
        events: Vec<WalletEvent>,
    ) -> Result<(Vec<WalletEvent>, bool), ScanError> {
        for t in targets {
            self.event_sender
                .send(ProcessingEvent::ScanStatus(ScanStatusEvent::Paused {
                    account_id: t.account.id,
                    last_scanned_height: t.resume_height,
                    reason: reason.clone(),
                }));
        }
        Ok((events, true))
    }

    fn emit_monitor_events(&self, account_id: i64, height: u64, res: MonitoringResult) {
        if !res.updated_displayed_transactions.is_empty() {
            self.event_sender
                .send(ProcessingEvent::TransactionsUpdated(TransactionsUpdatedEvent {
                    account_id,
                    updated_transactions: res.updated_displayed_transactions.clone(),
                }));
            self.event_sender
                .send(ProcessingEvent::TransactionsReady(DisplayedTransactionsEvent {
                    account_id,
                    transactions: res.updated_displayed_transactions,
                    block_height: Some(height),
                    is_initial_sync: false,
                }));
        }
    }

    async fn scan_blocks_with_timeout(
        &self,
        scanner: &mut HttpBlockchainScanner<KeyManager>,
        config: &ScanConfig,
    ) -> Result<(Vec<lightweight_wallet_libs::BlockScanResult>, bool), ScanError> {
        let mut timeout_retries = 0;
        let mut error_retries = 0;

        loop {
            match timeout(self.retry_config.timeout, scanner.scan_blocks(config)).await {
                Ok(Ok(result)) => return Ok(result),
                Ok(Err(e)) => {
                    error_retries += 1;
                    let error_msg = e.to_string();
                    warn!(
                        error = &*error_msg,
                        retry = error_retries,
                        max = self.retry_config.max_error_retries;
                        "Blockchain scan failed"
                    );
                    if error_retries >= self.retry_config.max_error_retries {
                        return Err(ScanError::Intermittent(e.to_string()));
                    }
                    let exponent = error_retries.min(MAX_BACKOFF_EXPONENT);
                    let backoff_secs = self
                        .retry_config
                        .error_backoff_base_secs
                        .pow(exponent)
                        .min(MAX_BACKOFF_SECONDS);
                    info!(
                        seconds = backoff_secs;
                        "Waiting before retrying..."
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                },
                Err(_) => {
                    timeout_retries += 1;
                    warn!(
                        retry = timeout_retries,
                        max = self.retry_config.max_timeout_retries;
                        "scan_blocks timed out"
                    );
                    if timeout_retries >= self.retry_config.max_timeout_retries {
                        return Err(ScanError::Timeout(timeout_retries));
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                },
            }
        }
    }

    fn emit_reorg_event(&self, account_id: i64, reorg_info: reorg::ReorgInformation, resume_height: u64) {
        self.event_sender
            .send(ProcessingEvent::ReorgDetected(ReorgDetectedEvent {
                account_id,
                reorg_from_height: reorg_info.rolled_back_from_height,
                new_height: resume_height,
                blocks_rolled_back: reorg_info.rolled_back_blocks_count,
                invalidated_output_hashes: reorg_info.invalidated_output_hashes,
                cancelled_transaction_ids: reorg_info.cancelled_transaction_ids,
                reorganized_displayed_transactions: reorg_info.reorganized_displayed_transactions,
            }));
    }
}
