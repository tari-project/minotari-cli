use std::time::Duration;

use anyhow::anyhow;
use tokio::{signal, sync::broadcast, time::sleep};

use tari_common::configuration::Network;

use crate::{
    api, db,
    scan::{self, ScanError},
    tasks::unlocker::TransactionUnlocker,
};

pub struct Daemon {
    password: String,
    base_url: String,
    database_file: String,
    max_blocks: u64,
    batch_size: u64,
    scan_interval: Duration,
    api_port: u16,
    network: Network,
    include_mempool: bool,
}

impl Daemon {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        password: String,
        base_url: String,
        database_file: String,
        max_blocks: u64,
        batch_size: u64,
        scan_interval_secs: u64,
        api_port: u16,
        network: Network,
        include_mempool: bool,
    ) -> Self {
        Self {
            password,
            base_url,
            database_file,
            max_blocks,
            batch_size,
            scan_interval: Duration::from_secs(scan_interval_secs),
            api_port,
            network,
            include_mempool,
        }
    }

    pub async fn run(&self) -> Result<(), ScanError> {
        println!("Daemon started. Press Ctrl+C to stop.");

        let (shutdown_tx, _) = broadcast::channel(1);

        let db_pool = db::init_db(&self.database_file).await.map_err(ScanError::Fatal)?;

        let unlocker = TransactionUnlocker::new(db_pool.clone());
        let unlocker_task_handle = unlocker.run(shutdown_tx.subscribe());

        let router = api::create_router(db_pool.clone(), self.network, self.password.clone());
        let addr = format!("0.0.0.0:{}", self.api_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ScanError::Fatal(anyhow!("Failed to bind API server to {}: {}", addr, e)))?;

        println!("API server listening on {}", addr);

        let mut shutdown_rx_api = shutdown_tx.subscribe();
        let api_server_handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    shutdown_rx_api.recv().await.ok();
                })
                .await
                .unwrap();
        });

        let shutdown_rx_scanner = shutdown_tx.subscribe();

        let shutdown_tx_clone = shutdown_tx.clone();
        let ctrlc_handle = tokio::spawn(async move {
            signal::ctrl_c().await.expect("Failed to listen for ctrl_c");
            println!("\nReceived shutdown signal, stopping all tasks...");
            let _ = shutdown_tx_clone.send(());
            Ok::<(), ()>(())
        });

        // HttpBlockchainScanner is marked as "NOT Send", so it is not possible to launch it in a new thread.
        // #[async_trait(?Send)]
        // impl<KM> BlockchainScanner for HttpBlockchainScanner<KM>
        let scanner_res = self.scan_and_sleep_loop(shutdown_rx_scanner).await;

        if let Err(e) = scanner_res {
            if shutdown_tx.send(()).is_err() {
                eprintln!("Failed to send shutdown signal. All tasks may not have received it.");
            }
            return Err(e);
        }

        if shutdown_tx.send(()).is_err() {
            eprintln!("Failed to send shutdown signal. All tasks may not have received it.");
        }

        let join_res = tokio::try_join!(api_server_handle, unlocker_task_handle, ctrlc_handle)
            .map_err(|e| ScanError::Fatal(anyhow!("A task panicked during shutdown: {}", e)))?;

        let (_api_res, _unlocker_res, _ctrlc_res) = join_res;

        println!("Daemon stopped gracefully.");
        Ok(())
    }

    async fn scan_and_sleep(&self) -> Result<(), ScanError> {
        println!("Starting wallet scan...");
        let result = scan::scan(
            &self.password,
            &self.base_url,
            &self.database_file,
            None, // Scan all accounts
            self.max_blocks,
            self.batch_size,
            self.include_mempool,
        )
        .await;

        match result {
            Ok(events) => {
                println!("Scan completed successfully. Found {} events.", events.len());
            },
            Err(e) => {
                println!("Scan failed: {}", e);
                return Err(e);
            },
        }

        sleep(self.scan_interval).await;
        Ok(())
    }

    async fn scan_and_sleep_loop(&self, mut shutdown_rx: broadcast::Receiver<()>) -> Result<(), ScanError> {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    println!("Scanner task received shutdown signal. Exiting gracefully.");
                    break;
                }
                res = self.scan_and_sleep() => {
                    if let Err(e) = res {
                        match e {
                            ScanError::Fatal(_) | ScanError::FatalSqlx(_) => {
                                println!("A fatal error occurred during the scan cycle: {}", e);
                                return Err(e);
                            },
                            ScanError::Intermittent(err_msg) => {
                                println!("An intermittent error occurred during the scan cycle: {}", err_msg);
                                sleep(self.scan_interval).await;
                            },
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
