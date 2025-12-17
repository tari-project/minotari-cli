#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Failed to parse output data: {0}")]
    ParseError(String),
}
