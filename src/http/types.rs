//! Type definitions for Tari base node RPC responses.
//!
//! This module contains the data structures used to deserialize JSON responses
//! from the Tari base node RPC API. These types model the blockchain state,
//! transaction submission results, and transaction query responses.
//!
//! # Overview
//!
//! The main types in this module are:
//!
//! - [`JsonRpcResponse`] - Generic wrapper for JSON-RPC 2.0 responses
//! - [`TipInfoResponse`] - Current blockchain tip information
//! - [`TxSubmissionResponse`] - Result of submitting a transaction
//! - [`TxQueryResponse`] - Status of a queried transaction
//!
//! # JSON-RPC Protocol
//!
//! The Tari base node uses JSON-RPC 2.0 for certain operations. Responses
//! are wrapped in [`JsonRpcResponse`] which contains either a `result` or
//! an `error` field, along with a request `id` for correlation.

use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// Generic wrapper for JSON-RPC 2.0 responses.
///
/// This struct represents the standard JSON-RPC 2.0 response format used
/// by the Tari base node for certain RPC methods (e.g., `submit_transaction`).
///
/// # Type Parameters
///
/// * `T` - The type of the successful result payload
///
/// # Fields
///
/// According to the JSON-RPC 2.0 specification, either `result` or `error`
/// should be present (but not both) in a valid response.
///
/// # Example
///
/// ```rust
/// use minotari::http::JsonRpcResponse;
/// use serde_json::json;
///
/// let json = json!({
///     "result": { "accepted": true },
///     "error": null,
///     "id": "1"
/// });
///
/// let response: JsonRpcResponse<serde_json::Value> =
///     serde_json::from_value(json).unwrap();
/// assert!(response.result.is_some());
/// assert!(response.error.is_none());
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse<T> {
    /// The successful result of the RPC call, if any.
    ///
    /// This field is `Some` when the request succeeded and `None` when
    /// an error occurred.
    pub result: Option<T>,

    /// Error message if the RPC call failed.
    ///
    /// This field is `Some` when the request failed and `None` on success.
    pub error: Option<String>,

    /// The request identifier, used to match responses with requests.
    ///
    /// This should match the `id` field from the corresponding request.
    pub id: String,
}

/// Response containing the current blockchain tip information.
///
/// This response is returned by the `/get_tip_info` endpoint and provides
/// information about the current state of the blockchain, including whether
/// the node is fully synchronized.
///
/// # Example
///
/// ```rust
/// use minotari::http::TipInfoResponse;
///
/// // A synced node with metadata
/// let tip_info = TipInfoResponse {
///     metadata: None, // Would contain ChainMetadata in practice
///     is_synced: true,
/// };
///
/// if tip_info.is_synced {
///     println!("Node is synchronized with the network");
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipInfoResponse {
    /// Detailed metadata about the blockchain tip.
    ///
    /// This may be `None` if the node is still initializing or if
    /// metadata is not available.
    pub metadata: Option<ChainMetadata>,

    /// Indicates whether the node is fully synchronized with the network.
    ///
    /// A value of `true` means the node has caught up with the network
    /// and is ready to process transactions. A value of `false` indicates
    /// the node is still syncing and may not have the latest state.
    pub is_synced: bool,
}

/// Metadata about the current state of the blockchain.
///
/// This struct contains detailed information about the blockchain tip,
/// including block height, hash, and pruning information. It is used
/// for monitoring node state and verifying synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainMetadata {
    /// The height of the best (most recent) block in the chain.
    ///
    /// This is the number of blocks from the genesis block to the tip.
    pub best_block_height: u64,

    /// The hash of the best block, as raw bytes.
    ///
    /// This uniquely identifies the current chain tip and can be used
    /// to detect chain reorganizations.
    pub best_block_hash: Vec<u8>,

    /// The pruning horizon in blocks.
    ///
    /// Blocks older than `best_block_height - pruning_horizon` may have
    /// been pruned and their full data may not be available.
    pub pruning_horizon: u64,

    /// The height up to which the chain has been pruned.
    ///
    /// Block data below this height may not be fully available.
    pub pruned_height: u64,

    /// The total accumulated proof-of-work difficulty.
    ///
    /// This represents the cumulative mining difficulty of all blocks
    /// in the chain and is used for chain selection.
    pub accumulated_difficulty: u64,

    /// The timestamp of the best block, in seconds since Unix epoch.
    pub timestamp: u64,
}

/// Response from submitting a transaction to the network.
///
/// This response indicates whether the transaction was accepted by the
/// base node and, if rejected, provides the reason for rejection.
///
/// # Example
///
/// ```rust
/// use minotari::http::{TxSubmissionResponse, TxSubmissionRejectionReason};
///
/// let response = TxSubmissionResponse {
///     accepted: true,
///     rejection_reason: TxSubmissionRejectionReason::None,
///     is_synced: true,
/// };
///
/// if response.accepted {
///     println!("Transaction accepted by the network");
/// } else {
///     println!("Transaction rejected: {}", response.rejection_reason);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TxSubmissionResponse {
    /// Whether the transaction was accepted by the base node.
    ///
    /// A value of `true` means the transaction passed validation and
    /// has been added to the mempool for mining.
    pub accepted: bool,

    /// The reason the transaction was rejected, if applicable.
    ///
    /// This field is [`TxSubmissionRejectionReason::None`] when the
    /// transaction is accepted.
    pub rejection_reason: TxSubmissionRejectionReason,

    /// Whether the base node is currently synchronized.
    ///
    /// If `false`, the submission result may not reflect the true
    /// network state, and the transaction should be resubmitted
    /// once the node is synced.
    pub is_synced: bool,
}

/// Reasons why a transaction submission may be rejected.
///
/// When a transaction is submitted to a base node, it undergoes validation.
/// If validation fails, one of these rejection reasons is returned to
/// indicate the cause of the failure.
///
/// # Display
///
/// This enum implements [`Display`] to provide human-readable rejection
/// messages suitable for logging or user interfaces.
///
/// # Example
///
/// ```rust
/// use minotari::http::TxSubmissionRejectionReason;
///
/// let reason = TxSubmissionRejectionReason::DoubleSpend;
/// println!("Rejection reason: {}", reason); // "Double Spend"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxSubmissionRejectionReason {
    /// No rejection - the transaction was accepted.
    None,

    /// The transaction has already been mined into a block.
    ///
    /// This typically occurs when resubmitting a transaction that
    /// has already been confirmed on the blockchain.
    AlreadyMined,

    /// The transaction attempts to spend outputs that have already been spent.
    ///
    /// This is a critical validation failure that indicates either
    /// a malicious double-spend attempt or a client-side bug.
    DoubleSpend,

    /// The transaction references inputs that are not in the UTXO set.
    ///
    /// This can occur if the referenced outputs do not exist or have
    /// not yet been confirmed.
    Orphan,

    /// The transaction has a time lock that has not yet expired.
    ///
    /// The transaction cannot be included in a block until the
    /// specified time or block height is reached.
    TimeLocked,

    /// The transaction failed validation rules.
    ///
    /// This is a general failure indicating that the transaction
    /// structure, signatures, or other properties are invalid.
    ValidationFailed,

    /// The transaction fee is too low to be accepted.
    ///
    /// The transaction does not meet the minimum fee requirements
    /// for inclusion in the mempool.
    FeeTooLow,
}

impl Display for TxSubmissionRejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxSubmissionRejectionReason::None => write!(f, "None"),
            TxSubmissionRejectionReason::AlreadyMined => write!(f, "Already Mined"),
            TxSubmissionRejectionReason::DoubleSpend => write!(f, "Double Spend"),
            TxSubmissionRejectionReason::Orphan => write!(f, "Orphan"),
            TxSubmissionRejectionReason::TimeLocked => write!(f, "Time Locked"),
            TxSubmissionRejectionReason::ValidationFailed => write!(f, "Validation Failed"),
            TxSubmissionRejectionReason::FeeTooLow => write!(f, "Fee Too Low"),
        }
    }
}

/// The location/status of a transaction in the network.
///
/// This enum represents where a transaction currently resides in the
/// transaction lifecycle, from unknown to fully mined.
///
/// # Variants
///
/// The variants are ordered by their integer discriminants, which can
/// be useful for comparison operations.
///
/// # Example
///
/// ```rust
/// use minotari::http::TxLocation;
///
/// let location = TxLocation::Mined;
/// match location {
///     TxLocation::Mined => println!("Transaction is confirmed"),
///     TxLocation::InMempool => println!("Transaction is pending"),
///     _ => println!("Transaction status unknown"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxLocation {
    /// The transaction location is unknown.
    ///
    /// This typically means the transaction was not found.
    None = 0,

    /// The transaction is known but not stored locally.
    ///
    /// The node has seen the transaction but has not retained it,
    /// possibly due to pruning or eviction from the mempool.
    NotStored = 1,

    /// The transaction is in the mempool awaiting mining.
    ///
    /// The transaction has been validated and accepted but has not
    /// yet been included in a block.
    InMempool = 2,

    /// The transaction has been mined into a block.
    ///
    /// The transaction is confirmed and included in the blockchain.
    /// Check `mined_height` in [`TxQueryResponse`] for the block height.
    Mined = 3,
}

/// Response from querying a transaction's status.
///
/// This response provides information about where a transaction is in
/// the network, including whether it has been mined and at what height.
///
/// # Example
///
/// ```rust
/// use minotari::http::{TxQueryResponse, TxLocation};
///
/// // Example of a mined transaction
/// let response = TxQueryResponse {
///     location: TxLocation::Mined,
///     mined_height: Some(123456),
///     mined_header_hash: Some(vec![0u8; 32]),
///     mined_timestamp: Some(1699900000),
/// };
///
/// if response.location == TxLocation::Mined {
///     println!("Transaction mined at height {:?}", response.mined_height);
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct TxQueryResponse {
    /// The current location of the transaction.
    ///
    /// See [`TxLocation`] for possible values.
    pub location: TxLocation,

    /// The block height where the transaction was mined, if applicable.
    ///
    /// This field is `Some` only when `location` is [`TxLocation::Mined`].
    pub mined_height: Option<u64>,

    /// The hash of the block header containing the transaction, if mined.
    ///
    /// This field is `Some` only when `location` is [`TxLocation::Mined`].
    pub mined_header_hash: Option<Vec<u8>>,

    /// The timestamp of the block containing the transaction, if mined.
    ///
    /// This is in seconds since Unix epoch and is `Some` only when
    /// `location` is [`TxLocation::Mined`].
    pub mined_timestamp: Option<u64>,
}
