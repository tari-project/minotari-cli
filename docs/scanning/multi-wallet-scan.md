# Multi-Wallet Scanning Architecture

## Overview

The Tari Wallet supports scanning multiple accounts (wallets) simultaneously using a single scanning thread. Instead of spawning a separate HTTP connection and thread for every account—which would be resource-intensive and redundant—the system uses a **Scan Coordinator**.

The Coordinator synchronizes the state of all wallets, identifies overlapping block ranges, and performs a single API request to fetch blocks that satisfy multiple wallets at once.

## Core Components

### 1. ScanCoordinator
The central engine (`coordinator.rs`). It holds a list of `AccountSyncTarget`s (the wallets being scanned) and manages the main loop. It calculates which block height to request next based on the lagging wallet.

### 2. AccountSyncTarget
Represents the state of a single wallet during the scan.
*   **`resume_height`**: The height of the **last successfully scanned block**.
*   **`key_manager`**: Used to attempt decryption of outputs in a block.
*   **`transaction_monitor`**: Tracks pending transaction confirmations.

### 3. ScannerStateManager
A helper that manages the underlying `HttpBlockchainScanner`. Its job is to detect if the set of "active accounts" has changed. If the set of keys required for the next batch is the same as the previous batch, it reuses the existing scanner connection to maintain state/performance.

## The Scanning Algorithm

The `unified_scan_loop` performs the following steps in every iteration:

### 1. Determine Global Horizon
The coordinator looks at all `AccountSyncTarget`s and finds the minimum `resume_height`.
*   `global_current_height` = `min(all_targets.resume_height)`

This ensures no wallet is left behind. The scan always starts from the oldest unscanned block among all accounts.

### 2. Identify Active Accounts (Overlap Detection)
The coordinator determines which accounts need blocks in the upcoming batch (e.g., next 50 blocks).
*   **Active:** If an account's `resume_height` is within `[global_current_height, global_current_height + batch_size]`.
*   **Inactive:** If an account is far ahead (e.g., a new wallet created at the tip, while an old wallet is syncing from genesis), it remains inactive for this batch.

### 3. Fetch Blocks
The `ScannerStateManager` constructs a scanner instance containing the keys of *only* the active accounts. It fetches a batch of blocks starting from `global_current_height`.

> **Note:** The underlying `HttpBlockchainScanner` is stateful. It "remembers" its cursor. The Coordinator manages this to ensure the cursor matches the `global_current_height`.

### 4. Distribute and Filter
The API returns a list of blocks. The Coordinator iterates through the **Active Accounts** and distributes the blocks.

**Crucial Logic:**
*   Each account filters the batch.
*   It **ignores** blocks where `block.height <= target.resume_height`.
*   It **processes** blocks where `block.height > target.resume_height`.

This filtering prevents "double-scanning" when multiple wallets are close to each other but not perfectly aligned.

### 5. Advance State
*   If an account successfully processes a block, its `resume_height` is updated to that block's height.
*   If the scanner reports `more_blocks: false` (Chain Tip reached), the coordinator switches to **Continuous/Polling** mode.

## Visualizing the "Catch-Up" Phase

Imagine **Wallet A** (Old) is at height 100, and **Wallet B** (New) is at height 1,000. The batch size is 100.

1.  **Batch 1 (100-200):** Global height is 100. Only Wallet A is active. Scanner fetches blocks 100-200. Wallet A processes them. Wallet B waits.
2.  **...**
3.  **Batch 10 (1000-1100):** Global height is 1000.
    *   Wallet A is at 1000.
    *   Wallet B is at 1000.
    *   **Overlap Detected:** Both wallets become active.
    *   Scanner fetches blocks 1000-1100 using keys for *both* A and B.
    *   Both wallets process the *same* downloaded block data to find their respective outputs.

## Handling the Chain Tip

When the scanner reaches the tip of the blockchain:
1.  The API may return the last block again (overlap) or an empty list, with `more_blocks: false`.
2.  The Coordinator detects this flag.
3.  It ensures any final blocks are processed.
4.  It enters a `wait_for_next_poll_cycle` (sleep).
5.  After the sleep, it checks for **Reorgs** and then resumes the loop.

## Reorg Detection

Reorgs are checked periodically (defined by `reorg_check_interval`).
1.  The Coordinator checks every account individually.
2.  If *any* account detects a reorg (local DB hash mismatch vs. Chain hash), the scan logic rolls back that specific account.
3.  Because `global_current_height` is calculated as the minimum of all accounts, the global scan automatically "rewinds" to accommodate the reorged account.

## Key Developer Notes

*   **Resume Height:** This value represents the **last successfully processed block**, not the "next" block to scan. This is vital for correct tip handling.
*   **Birthday Optimization:** New accounts are initialized with a `resume_height` based on their creation date (Birthday). They effectively "skip" the initial sync phase until the global scanner catches up to their birthday height.
*   **Thread Safety:** The scanning happens in a loop, but database writes (`ScanDbHandler`) are offloaded to blocking tasks to prevent locking the async runtime, as SQLite interactions are synchronous.
