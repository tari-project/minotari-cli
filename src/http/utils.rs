use tari_transaction_components::transaction_components::Transaction;

/// Maximum RPC frame size (4 MB)
const RPC_MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;

/// Size margin reserved for frame overhead and coinbase transactions
/// 10 KB for frame overhead + 2 MB for coinbase buffer
const SIZE_MARGIN: usize = (1024 * 10) + (2 * 1024 * 1024);

/// Maximum allowed transaction size after accounting for margin
const MAX_TRANSACTION_SIZE: usize = RPC_MAX_FRAME_SIZE - SIZE_MARGIN;

#[derive(Debug, Clone)]
pub struct TransactionTooLargeError {
    pub got: usize,
    pub max_allowed: usize,
}

impl std::fmt::Display for TransactionTooLargeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Transaction too large: got {} bytes, max allowed is {} bytes",
            self.got, self.max_allowed
        )
    }
}

impl std::error::Error for TransactionTooLargeError {}

/// Verify that the transaction is not too large to be broadcast.
pub fn check_transaction_size(transaction: &Transaction) -> Result<(), TransactionTooLargeError> {
    let serialized = serde_json::to_vec(transaction).map_err(|_| TransactionTooLargeError {
        got: 0,
        max_allowed: MAX_TRANSACTION_SIZE,
    })?;

    let size = serialized.len();

    if size > MAX_TRANSACTION_SIZE {
        Err(TransactionTooLargeError {
            got: size,
            max_allowed: MAX_TRANSACTION_SIZE,
        })
    } else {
        Ok(())
    }
}
