//! API request and response types for the Minotari wallet REST API.
//!
//! This module contains the data structures used for serializing and
//! deserializing API request bodies and response payloads. It includes
//! wrapper types for proper JSON representation of Tari-specific types.
//!
//! # Key Types
//!
//! - [`TariAddressBase58`]: Wrapper for Tari addresses with Base58 serialization
//! - [`LockFundsResult`]: Response type for the lock funds endpoint
//!
//! # Serialization Format
//!
//! All types in this module use JSON serialization via Serde. Numeric amounts
//! are represented as unsigned 64-bit integers (MicroMinotari), and addresses
//! use Base58 encoding for human-readable representation.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use tari_common_types::tari_address::TariAddress;
use tari_transaction_components::{tari_amount::MicroMinotari, transaction_components::WalletOutput};

/// A wrapper type for [`TariAddress`] with Base58 serialization.
///
/// This type provides JSON serialization and deserialization of Tari addresses
/// using Base58 encoding, which is the standard human-readable format for
/// Tari addresses.
///
/// # Serialization
///
/// When serialized to JSON, the address is represented as a Base58-encoded string:
///
/// ```json
/// "f4FxMqKAPDMqAjh6hTpCnLKfEu3MmS7NRu2YmKZPvZHc2K"
/// ```
///
/// # Deserialization
///
/// When deserializing from JSON, the string is parsed as a Base58-encoded
/// Tari address. Invalid addresses will result in a deserialization error.
///
/// # Example
///
/// ```rust,ignore
/// use crate::api::types::TariAddressBase58;
///
/// #[derive(Deserialize)]
/// struct Request {
///     recipient: TariAddressBase58,
/// }
///
/// // JSON: {"recipient": "f4FxMqKAPDMqAjh6hTpC..."}
/// ```
#[derive(Debug, Clone, PartialEq, Eq, utoipa::ToSchema)]
#[schema(value_type = String)]
pub struct TariAddressBase58(pub TariAddress);

/// Result of a successful fund locking operation.
///
/// This structure is returned by the `/accounts/{name}/lock_funds` endpoint
/// and contains all the information needed to construct a transaction using
/// the locked UTXOs.
///
/// # JSON Example
///
/// ```json
/// {
///   "utxos": [...],
///   "requires_change_output": true,
///   "total_value": 1500000,
///   "fee_without_change": 250,
///   "fee_with_change": 500
/// }
/// ```
///
/// # Usage
///
/// The locked UTXOs should be used as inputs for the transaction. If
/// `requires_change_output` is `true`, a change output must be created
/// to return excess funds. The appropriate fee (`fee_with_change` or
/// `fee_without_change`) should be used based on whether a change output
/// is included.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct LockFundsResult {
    /// The UTXOs that have been locked for this transaction.
    ///
    /// These outputs are reserved and cannot be used by other transactions
    /// until the lock expires or is released. Each UTXO contains the
    /// cryptographic data needed to spend it.
    #[schema(value_type = Vec<Object>)]
    pub utxos: Vec<WalletOutput>,

    /// Indicates whether a change output is required.
    ///
    /// If `true`, the total value of locked UTXOs exceeds the requested
    /// amount plus fees, and a change output should be created to return
    /// the excess to the wallet.
    pub requires_change_output: bool,

    /// The total value of all locked UTXOs in MicroMinotari.
    ///
    /// This is the sum of the values of all UTXOs in the `utxos` field.
    /// It will be greater than or equal to the requested amount plus the
    /// applicable fee.
    #[schema(value_type = u64)]
    pub total_value: MicroMinotari,

    /// The transaction fee if no change output is created.
    ///
    /// Use this fee when `requires_change_output` is `false`, or when
    /// intentionally burning the excess as a donation to miners.
    #[schema(value_type = u64)]
    pub fee_without_change: MicroMinotari,

    /// The transaction fee if a change output is included.
    ///
    /// This fee is higher than `fee_without_change` because the change
    /// output adds to the transaction size. Use this fee when
    /// `requires_change_output` is `true`.
    #[schema(value_type = u64)]
    pub fee_with_change: MicroMinotari,
}

/// Serializes a [`TariAddressBase58`] to its Base58 string representation.
///
/// # Output Format
///
/// The address is serialized as a plain string in Base58 encoding:
///
/// ```json
/// "f4FxMqKAPDMqAjh6hTpCnLKfEu3MmS7NRu2YmKZPvZHc2K"
/// ```
impl Serialize for TariAddressBase58 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_base58())
    }
}

/// Deserializes a [`TariAddressBase58`] from a Base58 string.
///
/// # Input Format
///
/// Expects a valid Tari address in Base58 encoding:
///
/// ```json
/// "f4FxMqKAPDMqAjh6hTpCnLKfEu3MmS7NRu2YmKZPvZHc2K"
/// ```
///
/// # Errors
///
/// Returns a deserialization error if:
/// - The string is not valid Base58 encoding
/// - The decoded data is not a valid Tari address
/// - The address checksum is invalid
impl<'de> Deserialize<'de> for TariAddressBase58 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TariAddress::from_base58(&s)
            .map(TariAddressBase58)
            .map_err(de::Error::custom)
    }
}
