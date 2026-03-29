use thiserror::Error;

#[derive(Error, Debug)]
pub enum MarketDataError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tungstenite::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Compression error: {0}")]
    Compression(String),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),
    
    #[error("Invalid subscription: {0}")]
    InvalidSubscription(String),
    
    #[error("Data integrity check failed")]
    DataIntegrityFailed,
    
    #[error("Historical data not available")]
    HistoricalDataUnavailable,
    
    #[error("Data source error: {0}")]
    DataSourceError(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Tokio error: {0}")]
    Tokio(#[from] tokio::task::JoinError),

    #[error("HFT protection violation: {0:?}")]
    HFTViolation(crate::hft_protection::HFTViolation),

    #[error("Circuit breaker triggered")]
    CircuitBreakerTriggered,
}

pub type Result<T> = std::result::Result<T, MarketDataError>;
