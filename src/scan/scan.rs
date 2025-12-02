use lightweight_wallet_libs::{HttpBlockchainScanner, ScanConfig, scanning::BlockchainScanner};
use sqlx::{Acquire, Sqlite, SqliteConnection, pool::PoolConnection};
use std::time::Duration;
use tari_transaction_components::key_manager::KeyManager;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use thiserror::Error;

use crate::{
    db,
    db::AccountRow,
    http::WalletHttpClient,
    models::WalletEvent,
    scan::{
        block_processor::{BlockProcessor, BlockProcessorError},
        events::{
            ChannelEventSender, EventSender, NoopEventSender, PauseReason, ProcessingEvent, ReorgDetectedEvent,
            ScanStatusEvent,
        },
        reorg,
    },
    transactions::{MonitoringState, TransactionMonitor},
};

const BIRTHDAY_GENESIS_FROM_UNIX_EPOCH: u64 = 1640995200;
const MAINNET_GENESIS_DATE: u64 = 1746489644;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("Fatal error: {0}")]
    Fatal(#[from] anyhow::Error),
    #[error("Intermittent error: {0}")]
    Intermittent(String),
}

impl From<sqlx::Error> for ScanError {
    fn from(e: sqlx::Error) -> Self {
        ScanError::Fatal(e.into())
    }
}

impl From<BlockProcessorError> for ScanError {
    fn from(e: BlockProcessorError) -> Self {
        ScanError::Fatal(e.into())
    }
}

struct ScanContext {
    scanner: HttpBlockchainScanner<KeyManager>,
    account_id: i64,
    scan_config: ScanConfig,
    reorg_check_interval: u64,
    wallet_client: WalletHttpClient,
    transaction_monitor: TransactionMonitor,
}

impl ScanContext {
    fn set_start_height(&mut self, height: u64) {
        self.scan_config = self.scan_config.clone().with_start_height(height);
    }
}

fn emit_reorg_event<E: EventSender>(
    event_sender: &E,
    account_id: i64,
    reorg_info: reorg::ReorgInformation,
    resume_height: u64,
) -> u64 {
    event_sender.send(ProcessingEvent::ReorgDetected(ReorgDetectedEvent {
        account_id,
        reorg_from_height: reorg_info.rolled_back_from_height,
        new_height: resume_height,
        blocks_rolled_back: reorg_info.rolled_back_blocks_count,
        invalidated_output_hashes: reorg_info.invalidated_output_hashes,
        cancelled_transaction_ids: reorg_info.cancelled_transaction_ids,
    }));
    resume_height
}

enum ContinuousWaitResult {
    Continue { resume_height: u64 },
    Cancelled,
}

async fn wait_for_next_poll_cycle<E: EventSender>(
    scanner_context: &mut ScanContext,
    conn: &mut PoolConnection<Sqlite>,
    event_sender: &E,
    poll_interval: Duration,
    last_scanned_height: u64,
    cancel_token: Option<&CancellationToken>,
) -> Result<ContinuousWaitResult, ScanError> {
    event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Waiting {
        account_id: scanner_context.account_id,
        resume_in: poll_interval,
    }));

    if let Some(token) = cancel_token {
        tokio::select! {
            _ = tokio::time::sleep(poll_interval) => {}
            _ = token.cancelled() => {
                event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Paused {
                    account_id: scanner_context.account_id,
                    last_scanned_height,
                    reason: PauseReason::Cancelled,
                }));
                return Ok(ContinuousWaitResult::Cancelled);
            }
        }
    } else {
        tokio::time::sleep(poll_interval).await;
    }

    let reorg_result = reorg::handle_reorgs(&mut scanner_context.scanner, conn, scanner_context.account_id)
        .await
        .map_err(ScanError::Fatal)?;

    let mut resume_height = last_scanned_height;

    if let Some(reorg_info) = reorg_result.reorg_information {
        println!(
            "Reorg detected at poll cycle start. Resetting from height {} to {}",
            last_scanned_height, reorg_result.resume_height
        );
        resume_height = emit_reorg_event(
            event_sender,
            scanner_context.account_id,
            reorg_info,
            reorg_result.resume_height,
        );
    }

    event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Started {
        account_id: scanner_context.account_id,
        from_height: resume_height,
    }));

    scanner_context.set_start_height(resume_height);

    Ok(ContinuousWaitResult::Continue { resume_height })
}

#[allow(clippy::too_many_arguments)]
async fn prepare_account_scan(
    account: &AccountRow,
    password: &str,
    base_url: &str,
    batch_size: u64,
    processing_threads: usize,
    reorg_check_interval: u64,
    monitoring_state: MonitoringState,
    conn: &mut SqliteConnection,
) -> Result<ScanContext, ScanError> {
    let key_manager = account.get_key_manager(password).await.map_err(ScanError::Fatal)?;

    let mut scanner = HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone(), processing_threads)
        .await
        .map_err(|e| ScanError::Intermittent(e.to_string()))?;

    let reorg_result = reorg::handle_reorgs(&mut scanner, conn, account.id)
        .await
        .map_err(ScanError::Fatal)?;

    let mut start_height = reorg_result.resume_height;

    if start_height == 0 {
        let birthday_day = (account.birthday as u64) * 24 * 60 * 60 + BIRTHDAY_GENESIS_FROM_UNIX_EPOCH;
        let estimate_birthday_block = (birthday_day.saturating_sub(MAINNET_GENESIS_DATE)) / 120;
        start_height = estimate_birthday_block;
    }

    let scan_config = ScanConfig::default()
        .with_start_height(start_height)
        .with_batch_size(batch_size);

    let wallet_client = WalletHttpClient::new(
        base_url
            .parse()
            .map_err(|e| ScanError::Fatal(anyhow::anyhow!("{}", e)))?,
    )
    .map_err(ScanError::Fatal)?;

    monitoring_state
        .initialize(conn, account.id)
        .await
        .map_err(ScanError::Fatal)?;

    let transaction_monitor = TransactionMonitor::new(monitoring_state);

    Ok(ScanContext {
        scanner,
        account_id: account.id,
        scan_config,
        reorg_check_interval,
        wallet_client,
        transaction_monitor,
    })
}

async fn run_scan_loop<E: EventSender + Clone>(
    scanner_context: &mut ScanContext,
    conn: &mut PoolConnection<Sqlite>,
    event_sender: E,
    mode: &ScanMode,
    cancel_token: Option<&CancellationToken>,
) -> Result<(Vec<WalletEvent>, bool), ScanError> {
    println!(
        "Starting scan for account {} from height {}",
        scanner_context.account_id, scanner_context.scan_config.start_height
    );
    let mut all_events = Vec::new();
    let mut total_scanned: u64 = 0;
    let mut blocks_since_reorg_check: u64 = 0;
    let initial_height = scanner_context.scan_config.start_height;
    let mut last_scanned_height = initial_height;

    event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Started {
        account_id: scanner_context.account_id,
        from_height: initial_height,
    }));

    loop {
        if let ScanMode::Partial { max_blocks } = mode
            && total_scanned >= *max_blocks
        {
            event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Paused {
                account_id: scanner_context.account_id,
                last_scanned_height,
                reason: PauseReason::MaxBlocksReached { limit: *max_blocks },
            }));
            return Ok((all_events, true));
        }

        if let Some(token) = cancel_token
            && token.is_cancelled()
        {
            event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Paused {
                account_id: scanner_context.account_id,
                last_scanned_height,
                reason: PauseReason::Cancelled,
            }));
            return Ok((all_events, true));
        }

        let (scanned_blocks, more_blocks) = scanner_context
            .scanner
            .scan_blocks(&scanner_context.scan_config)
            .await
            .map_err(|e| ScanError::Intermittent(e.to_string()))?;

        total_scanned += scanned_blocks.len() as u64;

        if scanned_blocks.is_empty() || !more_blocks {
            event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Completed {
                account_id: scanner_context.account_id,
                final_height: last_scanned_height,
                total_blocks_scanned: total_scanned,
            }));

            if let ScanMode::Continuous { poll_interval } = mode {
                match wait_for_next_poll_cycle(
                    scanner_context,
                    conn,
                    &event_sender,
                    *poll_interval,
                    last_scanned_height,
                    cancel_token,
                )
                .await?
                {
                    ContinuousWaitResult::Cancelled => return Ok((all_events, false)),
                    ContinuousWaitResult::Continue { resume_height } => {
                        last_scanned_height = resume_height;
                        total_scanned = 0;
                        continue;
                    },
                }
            }

            return Ok((all_events, false));
        }

        println!(
            "Processing {} scanned blocks for account {}",
            scanned_blocks.len(),
            scanner_context.account_id
        );
        for scanned_block in &scanned_blocks {
            let mut tx = conn.begin().await?;

            let mut processor = BlockProcessor::with_event_sender(scanner_context.account_id, event_sender.clone());
            processor.process_block(&mut tx, scanned_block).await?;

            all_events.extend(processor.into_wallet_events());

            tx.commit().await?;
        }

        if let Some(last_block) = scanned_blocks.last() {
            last_scanned_height = last_block.height;
            blocks_since_reorg_check += scanned_blocks.len() as u64;

            // Monitor pending transactions after processing blocks
            let monitor_events = scanner_context
                .transaction_monitor
                .monitor_if_needed(
                    &scanner_context.wallet_client,
                    conn,
                    scanner_context.account_id,
                    last_scanned_height,
                )
                .await
                .map_err(ScanError::Fatal)?;

            all_events.extend(monitor_events);
        }

        if more_blocks && blocks_since_reorg_check >= scanner_context.reorg_check_interval {
            let reorg_result = reorg::handle_reorgs(&mut scanner_context.scanner, conn, scanner_context.account_id)
                .await
                .map_err(ScanError::Fatal)?;

            if let Some(reorg_info) = reorg_result.reorg_information {
                println!(
                    "Reorg detected during scan. Resetting from height {} to {}",
                    last_scanned_height, reorg_result.resume_height
                );
                last_scanned_height = emit_reorg_event(
                    &event_sender,
                    scanner_context.account_id,
                    reorg_info,
                    reorg_result.resume_height,
                );
                scanner_context.set_start_height(reorg_result.resume_height);
            }

            blocks_since_reorg_check = 0;
        }

        db::prune_scanned_tip_blocks(conn, scanner_context.account_id, last_scanned_height)
            .await
            .map_err(ScanError::Fatal)?;

        event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::Progress {
            account_id: scanner_context.account_id,
            current_height: last_scanned_height,
            blocks_scanned: total_scanned,
        }));

        if more_blocks {
            event_sender.send(ProcessingEvent::ScanStatus(ScanStatusEvent::MoreBlocksAvailable {
                account_id: scanner_context.account_id,
                last_scanned_height,
            }));
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScanMode {
    Partial { max_blocks: u64 },
    Full,
    Continuous { poll_interval: Duration },
}

/// Builder for configuring and running blockchain scans.
pub struct Scanner {
    password: String,
    base_url: String,
    database_file: String,
    account_name: Option<String>,
    batch_size: u64,
    processing_threads: usize,
    reorg_check_interval: u64,
    mode: ScanMode,
    cancel_token: Option<CancellationToken>,
}

impl Scanner {
    pub fn new(password: &str, base_url: &str, database_file: &str, batch_size: u64) -> Self {
        Self {
            password: password.to_string(),
            base_url: base_url.to_string(),
            database_file: database_file.to_string(),
            account_name: None,
            batch_size,
            processing_threads: 8,
            reorg_check_interval: 1000,
            mode: ScanMode::Full,
            cancel_token: None,
        }
    }

    pub fn account(mut self, name: &str) -> Self {
        self.account_name = Some(name.to_string());
        self
    }

    /// Set the scan mode.
    ///
    /// Partial mode scans up to a maximum number of blocks.
    /// Full mode scans all available blocks once.
    /// Continuous mode keeps scanning for new blocks at regular intervals after hitting the chain tip.
    pub fn mode(mut self, mode: ScanMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn processing_threads(mut self, threads: usize) -> Self {
        self.processing_threads = threads;
        self
    }

    /// Set how often (in blocks) to check for reorgs during scanning.
    pub fn reorg_check_interval(mut self, interval: u64) -> Self {
        self.reorg_check_interval = interval;
        self
    }

    pub fn cancel_token(mut self, token: CancellationToken) -> Self {
        self.cancel_token = Some(token);
        self
    }

    /// Run the scan without real-time event streaming.
    pub async fn run(self) -> Result<(Vec<WalletEvent>, bool), ScanError> {
        self.run_internal(NoopEventSender).await
    }

    /// Run the scan with real-time event streaming.
    #[allow(clippy::type_complexity)]
    pub fn run_with_events(
        self,
    ) -> (
        mpsc::UnboundedReceiver<ProcessingEvent>,
        impl std::future::Future<Output = Result<(Vec<WalletEvent>, bool), ScanError>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_sender = ChannelEventSender::new(tx);
        let future = self.run_internal(event_sender);
        (rx, future)
    }

    async fn run_internal<E: EventSender + Clone>(
        self,
        event_sender: E,
    ) -> Result<(Vec<WalletEvent>, bool), ScanError> {
        let pool = db::init_db(&self.database_file).await.map_err(ScanError::Fatal)?;
        let mut conn = pool.acquire().await?;
        let mut all_events = Vec::new();
        let mut any_more_blocks = false;

        let account_name = self
            .account_name
            .clone()
            .ok_or_else(|| ScanError::Fatal(anyhow::anyhow!("Account name must be set")))?;

        let accounts = db::get_accounts(&mut conn, Some(&account_name)).await?;

        for account in accounts {
            let monitoring_state = MonitoringState::new();

            let mut scan_context = prepare_account_scan(
                &account,
                &self.password,
                &self.base_url,
                self.batch_size,
                self.processing_threads,
                self.reorg_check_interval,
                monitoring_state,
                &mut conn,
            )
            .await?;

            let (events, more_blocks) = run_scan_loop(
                &mut scan_context,
                &mut conn,
                event_sender.clone(),
                &self.mode,
                self.cancel_token.as_ref(),
            )
            .await?;

            all_events.extend(events);
            any_more_blocks = any_more_blocks || more_blocks;
        }

        Ok((all_events, any_more_blocks))
    }
}
