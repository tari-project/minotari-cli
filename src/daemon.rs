use std::time::Duration;

use anyhow::anyhow;
use tokio::time::sleep;

use crate::{
    api, db,
    scan::{self, ScanError},
};

pub struct Daemon {
    password: String,
    base_url: String,
    database_file: String,
    max_blocks: u64,
    batch_size: u64,
    scan_interval: Duration,
    api_port: u16,
}

impl Daemon {
    pub fn new(
        password: String,
        base_url: String,
        database_file: String,
        max_blocks: u64,
        batch_size: u64,
        scan_interval_secs: u64,
        api_port: u16,
    ) -> Self {
        Self {
            password,
            base_url,
            database_file,
            max_blocks,
            batch_size,
            scan_interval: Duration::from_secs(scan_interval_secs),
            api_port,
        }
    }

    pub async fn run(&self) -> Result<(), ScanError> {
        println!("Daemon started. Press Ctrl+C to stop.");

        let db_pool = db::init_db(&self.database_file).await.map_err(ScanError::Fatal)?;

        let router = api::create_router(db_pool);
        let addr = format!("0.0.0.0:{}", self.api_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ScanError::Fatal(anyhow!("Failed to bind API server to {}: {}", addr, e)))?;

        println!("API server listening on {}", addr);
        let api_server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nReceived shutdown signal, stopping daemon...");
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
        api_server.abort();
        println!("Daemon stopped.");
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
}
