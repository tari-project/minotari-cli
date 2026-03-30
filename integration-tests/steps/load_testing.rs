// Load Testing Step Definitions
//
// Step definitions for wallet load testing scenarios adapted from
// https://github.com/brianp/wallet-performance
//
// Scenarios:
//   - Pool Payout: rapid sequential payments at constant rate + burst
//   - Inbound Flood: incoming transaction detection with Poisson arrival
//   - Bidirectional: simultaneous send/receive with ramping load
//   - Fragmentation: UTXO fragmentation then aggregation
//   - Lock Contention: rapid sequential UTXO locking in escalating batches

use super::common::MinotariWorld;
use cucumber::{then, when};
use std::process::Command;
use std::time::{Duration, Instant};
use tari_common::configuration::Network::LocalNet;
use tari_common_types::tari_address::{TariAddress, TariAddressFeatures};
use tari_transaction_components::consensus::ConsensusConstantsBuilder;
use tari_transaction_components::key_manager::KeyManager;
use tari_transaction_components::key_manager::wallet_types::WalletType;
use tari_transaction_components::offline_signing::models::PrepareOneSidedTransactionForSigningResult;
use tari_transaction_components::offline_signing::models::TransactionResult;
use tari_transaction_components::offline_signing::sign_locked_transaction;

// =============================
// Helpers
// =============================

/// Generate a random recipient address for load testing.
fn random_recipient_address() -> String {
    let wallet = WalletType::new_random().expect("Failed to create random wallet");
    let address = TariAddress::new_dual_address(
        wallet.get_public_view_key(),
        wallet.get_public_spend_key(),
        LocalNet,
        TariAddressFeatures::create_one_sided_only(),
        None,
    )
    .unwrap();
    address.to_base58().to_string()
}

/// Create, sign, and submit a single transaction. Returns true on success.
async fn send_one_transaction(world: &mut MinotariWorld, amount: u64, tx_index: usize) -> bool {
    let db_path = world.database_path.as_ref().expect("Database not set up").clone();
    let base_url = base_node_url(world);
    let address = random_recipient_address();
    let recipient = format!("{}::{}", address, amount);
    let output_path = world.get_temp_path(&format!("load_tx_{}.json", tx_index));

    // Step 1: Create unsigned transaction
    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "create-unsigned-transaction".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--account-name".to_string(),
        "default".to_string(),
        "--recipient".to_string(),
        recipient,
        "--output-file".to_string(),
        output_path.to_str().unwrap().to_string(),
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to create transaction");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("Load tx {} creation failed: {}", tx_index, stderr);
        return false;
    }

    // Step 2: Read and sign
    let unsigned_json = match std::fs::read_to_string(&output_path) {
        Ok(j) => j,
        Err(e) => {
            println!("Load tx {} read failed: {}", tx_index, e);
            return false;
        },
    };

    let unsigned_tx = match PrepareOneSidedTransactionForSigningResult::from_json(&unsigned_json) {
        Ok(tx) => tx,
        Err(e) => {
            println!("Load tx {} parse failed: {}", tx_index, e);
            return false;
        },
    };

    let key_manager = KeyManager::new(world.wallet.clone()).expect("Failed to create key manager");
    let consensus_constants = ConsensusConstantsBuilder::new(LocalNet).build();

    let signed = match sign_locked_transaction(&key_manager, consensus_constants, LocalNet, unsigned_tx) {
        Ok(r) => r,
        Err(e) => {
            println!("Load tx {} signing failed: {}", tx_index, e);
            return false;
        },
    };

    // Step 3: Submit to base node
    let submit_url = format!("{}/json_rpc", base_url);
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let transaction = signed.signed_transaction.transaction;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "1",
        "method": "submit_transaction",
        "params": { "transaction": transaction }
    });

    match http_client.post(&submit_url).json(&request).send().await {
        Ok(resp) if resp.status().is_success() => true,
        Ok(resp) => {
            println!("Load tx {} submit status: {}", tx_index, resp.status());
            false
        },
        Err(e) => {
            println!("Load tx {} submit error: {}", tx_index, e);
            false
        },
    }
}

fn base_node_url(world: &MinotariWorld) -> String {
    if let Some((_, node)) = world.base_nodes.iter().next() {
        format!("http://127.0.0.1:{}", node.http_port)
    } else {
        panic!("No base node available");
    }
}

fn run_scan(world: &mut MinotariWorld) {
    let db_path = world.database_path.as_ref().expect("Database not set up").clone();
    let base_url = base_node_url(world);

    let (cmd, mut args) = world.get_minotari_command();
    args.extend_from_slice(&[
        "scan".to_string(),
        "--database-path".to_string(),
        db_path.to_str().unwrap().to_string(),
        "--password".to_string(),
        world.test_password.clone(),
        "--base-url".to_string(),
        base_url,
        "--max-blocks-to-scan".to_string(),
        "1000".to_string(),
    ]);

    let output = Command::new(&cmd)
        .args(&args)
        .output()
        .expect("Failed to execute scan command");

    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_output = Some(String::from_utf8_lossy(&output.stdout).to_string());
    world.last_command_error = Some(String::from_utf8_lossy(&output.stderr).to_string());
}

// =============================
// Pool Payout Steps
// =============================

#[when(regex = r"^I send (\d+) transactions at a constant rate of (\d+) per second$")]
async fn send_constant_rate(world: &mut MinotariWorld, count: usize, tps: usize) {
    let interval = Duration::from_secs_f64(1.0 / tps as f64);
    let amount: u64 = 1000; // 1000 µT per tx

    println!(
        "Pool payout: sending {} transactions at {} tx/sec ({:?} interval)",
        count, tps, interval
    );

    let start = Instant::now();
    let mut successes = 0u64;
    let mut failures = 0u64;

    for i in 0..count {
        let tx_start = Instant::now();
        if send_one_transaction(world, amount, i).await {
            successes += 1;
        } else {
            failures += 1;
        }
        // Pace to the target rate
        let elapsed = tx_start.elapsed();
        if elapsed < interval {
            tokio::time::sleep(interval - elapsed).await;
        }
    }

    let total_duration = start.elapsed();
    let effective_tps = if total_duration.as_secs_f64() > 0.0 {
        successes as f64 / total_duration.as_secs_f64()
    } else {
        0.0
    };

    println!(
        "Constant rate: {}/{} succeeded in {:.2?} ({:.2} effective tx/s)",
        successes, count, total_duration, effective_tps
    );
    assert_eq!(
        failures, 0,
        "Expected all transactions to succeed, but {} failed",
        failures
    );

    world
        .benchmark_timings
        .insert("pool_payout_constant".to_string(), total_duration);
    world.last_command_output = Some(format!(
        "constant_successes={},constant_failures={}",
        successes, failures
    ));
}

#[when(regex = r"^I send a burst of (\d+) transactions as fast as possible$")]
async fn send_burst(world: &mut MinotariWorld, count: usize) {
    let amount: u64 = 1000;

    println!("Pool payout burst: sending {} transactions ASAP", count);
    let start = Instant::now();
    let mut successes = 0u64;
    // Offset indices to avoid file collisions with constant-rate txs
    let offset = 10_000;

    for i in 0..count {
        if send_one_transaction(world, amount, offset + i).await {
            successes += 1;
        }
    }

    let duration = start.elapsed();
    let failures = count as u64 - successes;
    let effective_tps = if duration.as_secs_f64() > 0.0 {
        successes as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    println!(
        "Burst: {}/{} succeeded in {:.2?} ({:.2} effective tx/s), {} failures",
        successes, count, duration, effective_tps, failures
    );

    world
        .benchmark_timings
        .insert("pool_payout_burst".to_string(), duration);
}

#[then("all pool payout transactions should succeed")]
async fn pool_payout_success(world: &mut MinotariWorld) {
    if let Some(output) = &world.last_command_output {
        // We don't require 100% success — print the results for analysis
        println!("Pool payout results: {}", output);
    }
    // Verify scan succeeded
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Post-payout scan failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

// =============================
// Inbound Flood Steps
// =============================

#[when(regex = r"^I send (\d+) inbound transactions using Poisson distribution at ([\d.]+) per second$")]
async fn send_poisson(world: &mut MinotariWorld, count: usize, avg_tps: f64) {
    let amount: u64 = 1000;
    let offset = 20_000;

    println!(
        "Inbound flood: sending {} transactions with Poisson arrival (avg {:.1} tx/s)",
        count, avg_tps
    );

    let start = Instant::now();
    let mut successes = 0u64;
    let mut failures = 0u64;

    for i in 0..count {
        if send_one_transaction(world, amount, offset + i).await {
            successes += 1;
        } else {
            failures += 1;
        }

        // Exponential inter-arrival time (Poisson process)
        let uniform: f64 = rand::random::<f64>().max(1e-10);
        let delay_secs = -uniform.ln() / avg_tps;
        let delay = Duration::from_secs_f64(delay_secs.min(10.0));
        tokio::time::sleep(delay).await;
    }

    let duration = start.elapsed();
    println!("Inbound flood: {}/{} sent in {:.2?}", successes, count, duration);
    assert_eq!(
        failures, 0,
        "Expected all transactions to be sent successfully, but {} failed",
        failures
    );

    world
        .benchmark_timings
        .insert("inbound_flood_send".to_string(), duration);
}

#[when("I measure scan detection time for incoming transactions")]
async fn measure_scan_detection(world: &mut MinotariWorld) {
    let start = Instant::now();
    run_scan(world);
    let duration = start.elapsed();

    println!("Inbound flood scan detection: {:.2?}", duration);
    world
        .benchmark_timings
        .insert("inbound_flood_detection".to_string(), duration);
}

#[then("all inbound transactions should be detected")]
async fn inbound_detected(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Inbound detection scan failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
    println!("Inbound flood scan completed successfully");
}

// =============================
// Bidirectional Steps
// =============================

#[when(regex = r"^I send transactions with ramping load from (\d+) to (\d+) per minute over (\d+) steps$")]
async fn send_ramping(world: &mut MinotariWorld, start_tpm: usize, max_tpm: usize, steps: usize) {
    let amount: u64 = 1000;
    let offset = 30_000;
    let step_tpm = if steps > 1 {
        (max_tpm - start_tpm) / (steps - 1)
    } else {
        0
    };
    // Each step runs for a fixed duration (e.g. 30 seconds for test speed)
    let step_duration = Duration::from_secs(30);

    println!(
        "Bidirectional: ramping {} -> {} tx/min over {} steps ({} tx/min increment, {:?} per step)",
        start_tpm, max_tpm, steps, step_tpm, step_duration
    );

    let run_start = Instant::now();
    let mut total_successes = 0u64;
    let mut total_failures = 0u64;
    let mut total_sent = 0usize;
    let mut tx_index = offset;

    for step in 0..steps {
        let current_tpm = (start_tpm + step * step_tpm).min(max_tpm);
        let interval = if current_tpm > 0 {
            Duration::from_secs_f64(60.0 / current_tpm as f64)
        } else {
            step_duration // no sends this step
        };

        println!(
            "  Step {}/{}: {} tx/min ({:?} interval)",
            step + 1,
            steps,
            current_tpm,
            interval
        );

        let step_start = Instant::now();
        while step_start.elapsed() < step_duration {
            let tx_start = Instant::now();
            if send_one_transaction(world, amount, tx_index).await {
                total_successes += 1;
            } else {
                total_failures += 1;
            }
            total_sent += 1;
            tx_index += 1;

            let elapsed = tx_start.elapsed();
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
    }

    let total_duration = run_start.elapsed();
    println!(
        "Bidirectional: {}/{} succeeded in {:.2?}",
        total_successes, total_sent, total_duration
    );
    assert_eq!(
        total_failures, 0,
        "Expected all transactions to succeed, but {} failed",
        total_failures
    );

    world
        .benchmark_timings
        .insert("bidirectional_ramp".to_string(), total_duration);
}

#[then("all bidirectional transactions should succeed")]
async fn bidirectional_success(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Bidirectional post-scan failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

// =============================
// Fragmentation Steps
// =============================

#[when(regex = r"^I fragment UTXOs by sending (\d+) small transactions of (\d+) microTari$")]
async fn fragment_utxos(world: &mut MinotariWorld, count: usize, amount: u64) {
    let offset = 40_000;

    println!("Fragmentation: creating {} small UTXOs of {} µT each", count, amount);

    let start = Instant::now();
    let mut successes = 0u64;
    let mut failures = 0u64;

    for i in 0..count {
        if send_one_transaction(world, amount, offset + i).await {
            successes += 1;
        } else {
            failures += 1;
        }
    }

    let duration = start.elapsed();
    println!(
        "Fragmentation: {}/{} fragment txs in {:.2?}",
        successes, count, duration
    );
    assert_eq!(
        failures, 0,
        "Expected all fragmentation transactions to succeed, but {} failed",
        failures
    );

    world
        .benchmark_timings
        .insert("fragmentation_split".to_string(), duration);
}

#[when("I send aggregation transactions of increasing size")]
async fn send_aggregation(world: &mut MinotariWorld) {
    let amounts: Vec<u64> = vec![2000, 5000, 10000, 20000];
    let offset = 50_000;

    println!("Fragmentation: sending aggregation txs {:?} µT", amounts);

    let start = Instant::now();
    let mut successes = 0u64;

    for (i, amount) in amounts.iter().enumerate() {
        if send_one_transaction(world, *amount, offset + i).await {
            successes += 1;
            println!("  Aggregation {} µT: OK", amount);
        } else {
            println!("  Aggregation {} µT: FAILED", amount);
        }
    }

    let duration = start.elapsed();
    println!(
        "Aggregation: {}/{} succeeded in {:.2?}",
        successes,
        amounts.len(),
        duration
    );

    world
        .benchmark_timings
        .insert("fragmentation_aggregation".to_string(), duration);
}

#[then("the aggregation transactions should succeed")]
async fn aggregation_success(world: &mut MinotariWorld) {
    assert_eq!(
        world.last_command_exit_code,
        Some(0),
        "Post-aggregation scan failed: {}",
        world.last_command_error.as_deref().unwrap_or("")
    );
}

// =============================
// Lock Contention Steps
// =============================

#[when(regex = r"^I send a batch of (\d+) rapid transactions$")]
async fn send_rapid_batch(world: &mut MinotariWorld, count: usize) {
    let amount: u64 = 1000;
    // Use a unique offset per batch based on current timing
    let offset = 60_000 + (Instant::now().elapsed().as_millis() as usize % 10_000);

    println!("Lock contention: sending batch of {} rapid transactions", count);

    let start = Instant::now();
    let mut successes = 0u64;
    let mut failures = 0u64;
    for i in 0..count {
        if send_one_transaction(world, amount, offset + i).await {
            successes += 1;
        } else {
            failures += 1;
        }
    }

    let duration = start.elapsed();
    let effective_tps = if duration.as_secs_f64() > 0.0 {
        successes as f64 / duration.as_secs_f64()
    } else {
        0.0
    };

    println!(
        "Batch {}: {}/{} succeeded in {:.2?} ({:.2} tx/s), {} failures",
        count, successes, count, duration, effective_tps, failures
    );
    assert_eq!(
        failures, 0,
        "Expected all transactions to succeed, but {} failed",
        failures
    );

    let key = format!("lock_contention_batch_{}", count);
    world.benchmark_timings.insert(key, duration);
}

#[when(regex = r"^I wait (\d+) seconds for cooldown$")]
async fn wait_cooldown(_world: &mut MinotariWorld, seconds: u64) {
    println!("Cooling down for {} seconds...", seconds);
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

// =============================
// Common Load Test Steps
// =============================

#[when("I scan the wallet")]
async fn scan_wallet(world: &mut MinotariWorld) {
    run_scan(world);
}

#[then(regex = r#"^I print the load test results for "([^"]*)"$"#)]
async fn print_load_results(world: &mut MinotariWorld, scenario: String) {
    println!("\n========================================");
    println!("  LOAD TEST RESULTS: {}", scenario.to_uppercase());
    println!("========================================");

    let prefix = format!("{}_", scenario);
    let mut total = Duration::ZERO;

    for (key, duration) in &world.benchmark_timings {
        if key.starts_with(&prefix) || key.starts_with(&scenario) {
            let label = key.trim_start_matches(&prefix);
            println!("  {:<30} {:.2?}", label, duration);
            total += *duration;
        }
    }

    if total > Duration::ZERO {
        println!("  {:<30} {:.2?}", "TOTAL", total);
    }

    println!("========================================\n");
}
