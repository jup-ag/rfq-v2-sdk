//! Type definitions and helpers for the RFQv2 SDK

// Re-export the generated types for convenience
pub use crate::market_maker::{
    Cluster, GetAllOrderbooksRequest, GetAllOrderbooksResponse, GetQuotesRequest,
    GetQuotesResponse, MarketMakerQuote, MarketMakerSwap, Orderbook, PriceLevel, QuoteUpdate,
    SequenceNumberRequest, SequenceNumberResponse, SwapMessageType, SwapUpdate, Token, TokenPair,
    UpdateType,
};

/// Configuration for connecting to the RFQv2 service
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Service endpoint URL
    pub endpoint: String,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
    /// Authentication token for API access
    pub auth_token: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:2408".to_string(),
            timeout_secs: crate::DEFAULT_TIMEOUT_SECS,
            auth_token: None,
        }
    }
}

impl ClientConfig {
    /// Create a new configuration with the specified endpoint
    pub fn new<S: Into<String>>(endpoint: S) -> Self {
        Self {
            endpoint: endpoint.into(),
            ..Default::default()
        }
    }

    /// Set the connection timeout
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    /// Set the authentication token for API access
    pub fn with_auth_token<S: Into<String>>(mut self, auth_token: S) -> Self {
        self.auth_token = Some(auth_token.into());
        self
    }
}

/// Common token pairs for convenience
impl TokenPair {
    /// SOL/USDC token pair on mainnet
    pub fn sol_usdc() -> Self {
        Self {
            base_token: Token {
                address: "So11111111111111111111111111111111111111112".to_string(),
                decimals: 9,
                symbol: "SOL".to_string(),
                owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            },
            quote_token: Token {
                address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                decimals: 6,
                symbol: "USDC".to_string(),
                owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            },
        }
    }

    /// Create a custom token pair
    pub fn new(base_token: Token, quote_token: Token) -> Self {
        Self {
            base_token,
            quote_token,
        }
    }
}

impl Token {
    /// Create a new token
    pub fn new<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        address: S1,
        decimals: u32,
        symbol: S2,
        owner: S3,
    ) -> Self {
        Self {
            address: address.into(),
            decimals,
            symbol: symbol.into(),
            owner: owner.into(),
        }
    }
}

impl PriceLevel {
    /// Create a new price level
    pub fn new(volume: u64, price: u64) -> Self {
        Self { volume, price }
    }
}
