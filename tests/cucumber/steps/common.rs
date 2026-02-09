// Common World Definition and Utilities for Cucumber BDD Tests
//
// This module contains the shared state object (MinotariWorld) and common
// step definitions used across multiple test scenarios.

use cucumber::{given, World};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

// =============================
// World Definition
// =============================

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct MinotariWorld {
    pub temp_dir: Option<TempDir>,
    pub database_path: Option<PathBuf>,
    pub output_file: Option<PathBuf>,
    pub wallet_data: Option<serde_json::Value>,
    pub last_command_output: Option<String>,
    pub last_command_error: Option<String>,
    pub last_command_exit_code: Option<i32>,
    pub test_view_key: String,
    pub test_spend_key: String,
    pub test_password: String,
    pub daemon_handle: Option<std::process::Child>,
    pub api_port: Option<u16>,
    pub locked_funds: HashMap<String, serde_json::Value>,
}

impl MinotariWorld {
    pub fn new() -> Self {
        Self {
            temp_dir: None,
            database_path: None,
            output_file: None,
            wallet_data: None,
            last_command_output: None,
            last_command_error: None,
            last_command_exit_code: None,
            test_view_key: "0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            test_spend_key: "0000000000000000000000000000000000000000000000000000000000000002".to_string(),
            test_password: "test_password_minimum_32_chars_long_for_encryption".to_string(),
            daemon_handle: None,
            api_port: None,
            locked_funds: HashMap::new(),
        }
    }

    pub fn setup_temp_dir(&mut self) {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        self.temp_dir = Some(temp_dir);
    }

    pub fn get_temp_path(&self, filename: &str) -> PathBuf {
        self.temp_dir
            .as_ref()
            .expect("Temp directory not set up")
            .path()
            .join(filename)
    }

    pub fn setup_database(&mut self) {
        if self.temp_dir.is_none() {
            self.setup_temp_dir();
        }
        let db_path = self.get_temp_path("test_wallet.db");
        self.database_path = Some(db_path);
    }

    pub fn cleanup(&mut self) {
        if let Some(mut child) = self.daemon_handle.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.temp_dir = None;
    }
}

impl Drop for MinotariWorld {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// =============================
// Common Step Definitions
// =============================

#[given("I have a clean test environment")]
async fn clean_environment(world: &mut MinotariWorld) {
    world.setup_temp_dir();
}

#[given("I have a test database")]
async fn setup_database(world: &mut MinotariWorld) {
    world.setup_database();
}

#[given("I have a test database with an existing wallet")]
async fn database_with_wallet(world: &mut MinotariWorld) {
    world.setup_database();
    let db_path = world.database_path.as_ref().expect("Database not set up");
    
    let _ = Command::new("cargo")
        .args(&[
            "run", "--bin", "minotari", "--", "import-view-key",
            "--view-private-key", &world.test_view_key,
            "--spend-public-key", &world.test_spend_key,
            "--password", &world.test_password,
            "--database-path", db_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to set up test wallet");
}

#[given("I have a test database with multiple accounts")]
async fn database_with_multiple_accounts(world: &mut MinotariWorld) {
    database_with_wallet(world).await;
}

#[given("I have a test database with a new wallet")]
async fn database_with_new_wallet(world: &mut MinotariWorld) {
    database_with_wallet(world).await;
}
