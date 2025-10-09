use std::time::Duration;

use tokio::time::sleep;

use crate::scan;

pub struct Daemon {
    password: String,
    base_url: String,
    database_file: String,
    max_blocks: u64,
    batch_size: u64,
    scan_interval: Duration,
}

impl Daemon {
    pub fn new(
        password: String,
        base_url: String,
        database_file: String,
        max_blocks: u64,
        batch_size: u64,
        scan_interval_secs: u64,
    ) -> Self {
        Self {
            password,
            base_url,
            database_file,
            max_blocks,
            batch_size,
            scan_interval: Duration::from_secs(scan_interval_secs),
        }
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        println!("Daemon started. Press Ctrl+C to stop.");

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nReceived shutdown signal, stopping daemon...");
                    break;
                }
                res = self.scan_and_sleep() => {
                    if let Err(e) = res {
                        println!("An error occurred during the scan cycle: {}", e);
                        sleep(self.scan_interval).await;
                    }
                }
            }
        }
        println!("Daemon stopped.");
        Ok(())
    }

    async fn scan_and_sleep(&self) -> Result<(), anyhow::Error> {
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
