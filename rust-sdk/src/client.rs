//! Main client implementation for the RFQv2 SDK

use crate::error::{MarketMakerError, Result};
use crate::market_maker::market_maker_ingestion_service_client::MarketMakerIngestionServiceClient;
use crate::streaming::{QuoteStreamHandle, SwapStreamHandle};
use crate::types::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::Request;
use tracing::{debug, error, info, instrument, warn};

/// Main client for interacting with the RFQv2
#[derive(Clone)]
pub struct MarketMakerClient {
    inner: MarketMakerIngestionServiceClient<Channel>,
    config: ClientConfig,
}

impl MarketMakerClient {
    /// Helper to add authentication token to a request
    fn add_auth_token<T>(&self, mut request: Request<T>) -> Result<Request<T>> {
        if let Some(auth_token) = &self.config.auth_token {
            request.metadata_mut().insert(
                "x-api-key",
                auth_token
                    .parse()
                    .map_err(|_| MarketMakerError::configuration("Invalid auth token format"))?,
            );
            debug!("Added authentication token to request metadata");
        }
        Ok(request)
    }

    /// Connect to the RFQv2 service with default configuration
    #[instrument(skip(endpoint))]
    pub async fn connect<S: Into<String>>(endpoint: S) -> Result<Self> {
        Self::connect_with_config(ClientConfig::new(endpoint.into())).await
    }

    /// Connect to the RFQv2 service with custom configuration
    #[instrument(skip(config))]
    pub async fn connect_with_config(config: ClientConfig) -> Result<Self> {
        info!("Connecting to RFQv2 service at {}", config.endpoint);

        // Ensure a rustls CryptoProvider is available for TLS connections
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut endpoint = Endpoint::try_from(config.endpoint.clone())
            .map_err(|e| MarketMakerError::configuration(format!("Invalid endpoint: {}", e)))?
            .timeout(Duration::from_secs(config.timeout_secs))
            // HTTP/2 keepalive: send PINGs every 10s to prevent load balancers
            // and reverse proxies from dropping idle streaming connections.
            .http2_keep_alive_interval(Duration::from_secs(10))
            // If the server does not respond to a keepalive PING within 20s,
            // consider the connection dead.
            .keep_alive_timeout(Duration::from_secs(20))
            // Send keepalive PINGs even when there are no active RPCs. This is
            // critical for long-lived bidirectional streams that may have idle
            // periods on one direction.
            .keep_alive_while_idle(true)
            // Enable TCP keepalive as a secondary safeguard.
            .tcp_keepalive(Some(Duration::from_secs(60)));

        if config.endpoint.starts_with("https://") {
            debug!("Configuring HTTPS connection with HTTP/2 over TLS and ALPN");
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(|e| {
                    MarketMakerError::configuration(format!("TLS configuration failed: {}", e))
                })?;
        } else {
            debug!("Using HTTP/2 connection (plain text for development)");
        }

        let channel = endpoint.connect().await.map_err(|e| {
            error!("Connection error: {}", e);
            MarketMakerError::Connection(e)
        })?;

        debug!("Successfully connected to RFQv2 service with HTTP/2 protocol");

        Ok(Self {
            inner: MarketMakerIngestionServiceClient::new(channel),
            config,
        })
    }

    /// Start a bidirectional gRPC streaming connection for real-time quote updates
    #[instrument(skip(self))]
    pub async fn start_streaming(&mut self) -> Result<QuoteStreamHandle> {
        info!("Starting bidirectional gRPC streaming connection");

        let (quote_tx, quote_rx) = mpsc::unbounded_channel();
        let request = self.add_auth_token(Request::new(UnboundedReceiverStream::new(quote_rx)))?;

        let response = self
            .inner
            .stream_quotes(request)
            .await
            .map_err(MarketMakerError::Grpc)?;

        debug!("gRPC streaming connection established successfully");

        Ok(QuoteStreamHandle::new(quote_tx, response.into_inner()))
    }

    /// Start a bidirectional gRPC streaming connection for swap updates
    #[instrument(skip(self))]
    pub async fn start_swap_streaming(&mut self) -> Result<SwapStreamHandle> {
        info!("Starting bidirectional gRPC swap streaming connection");

        let (swap_tx, swap_rx) = mpsc::unbounded_channel();
        let request = self.add_auth_token(Request::new(UnboundedReceiverStream::new(swap_rx)))?;

        let response = self
            .inner
            .stream_swap(request)
            .await
            .map_err(MarketMakerError::Grpc)?;

        debug!("gRPC swap streaming connection established successfully");

        Ok(SwapStreamHandle::new(swap_tx, response.into_inner()))
    }

    /// Start streaming with automatic sequence number synchronization.
    ///
    /// Returns the stream handle and the sequence number the first quote should use.
    #[instrument(skip(self), fields(maker_id = %maker_id))]
    pub async fn start_streaming_with_sync(
        &mut self,
        maker_id: String,
        auth_token: String,
    ) -> Result<(QuoteStreamHandle, u64)> {
        let last_sequence = self
            .get_last_sequence_number(maker_id.clone(), auth_token)
            .await?;
        let stream_handle = self.start_streaming().await?;

        debug!(
            "Sequence sync complete for maker {}: last={}, next={}",
            maker_id,
            last_sequence,
            last_sequence + 1
        );

        Ok((stream_handle, last_sequence + 1))
    }

    /// Get a copy of the client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Get the last sequence number for a maker (for synchronization before streaming)
    #[instrument(skip(self), fields(maker_id = %maker_id))]
    pub async fn get_last_sequence_number(
        &mut self,
        maker_id: String,
        auth_token: String,
    ) -> Result<u64> {
        debug!("Getting last sequence number for maker: {}", maker_id);
        let request = Request::new(SequenceNumberRequest {
            maker_id: maker_id.clone(),
            auth_token,
        });

        let response = self
            .inner
            .get_last_sequence_number(request)
            .await
            .map_err(MarketMakerError::Grpc)?
            .into_inner();

        if response.success {
            debug!(
                "Retrieved last sequence number for maker {}: {}",
                maker_id, response.last_sequence_number
            );
            Ok(response.last_sequence_number)
        } else {
            warn!(
                "Failed to get sequence number for maker {}: {}",
                maker_id, response.message
            );
            Ok(0)
        }
    }

    /// Get quotes for a specific token pair
    #[instrument(skip(self))]
    pub async fn get_quotes(
        &mut self,
        token_pair: TokenPair,
        auth_token: String,
    ) -> Result<GetQuotesResponse> {
        debug!("Getting quotes for token pair");
        let request = Request::new(GetQuotesRequest {
            token_pair,
            auth_token,
        });

        let response = self
            .inner
            .get_quotes(request)
            .await
            .map_err(MarketMakerError::Grpc)?
            .into_inner();

        info!("Retrieved {} quotes", response.quotes.len());

        Ok(response)
    }

    /// Get all orderbooks for a specific cluster, or all clusters when `None`
    #[instrument(skip(self))]
    pub async fn get_all_orderbooks(
        &mut self,
        cluster: Option<Cluster>,
    ) -> Result<GetAllOrderbooksResponse> {
        debug!("Getting all orderbooks");
        let request = self.add_auth_token(Request::new(GetAllOrderbooksRequest {
            cluster: cluster.map(|c| c as i32),
        }))?;

        let response = self
            .inner
            .get_all_orderbooks(request)
            .await
            .map_err(MarketMakerError::Grpc)?
            .into_inner();

        info!(
            "Retrieved {} orderbooks at timestamp {}",
            response.orderbooks.len(),
            response.timestamp
        );

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_maker::market_maker_ingestion_service_server::{
        MarketMakerIngestionService, MarketMakerIngestionServiceServer,
    };
    use crate::market_maker::*;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::{Request, Response, Status};

    /// Minimal mock server implementing GetQuotes and StreamQuotes
    struct MockService;

    #[tonic::async_trait]
    impl MarketMakerIngestionService for MockService {
        async fn get_last_sequence_number(
            &self,
            _req: Request<SequenceNumberRequest>,
        ) -> std::result::Result<Response<SequenceNumberResponse>, Status> {
            unimplemented!()
        }

        /// Mirrors the ingestion service: the endpoint is authenticated.
        async fn get_all_orderbooks(
            &self,
            req: Request<GetAllOrderbooksRequest>,
        ) -> std::result::Result<Response<GetAllOrderbooksResponse>, Status> {
            req.metadata()
                .get("x-api-key")
                .ok_or_else(|| Status::unauthenticated("Missing authentication token"))?;

            Ok(Response::new(GetAllOrderbooksResponse {
                orderbooks: vec![],
                timestamp: 1_000_000,
            }))
        }

        type StreamQuotesStream = ReceiverStream<std::result::Result<QuoteUpdate, Status>>;

        /// Echo one `QuoteUpdate` per inbound quote: NEW when the quote carries
        /// levels, REJECTED otherwise.
        async fn stream_quotes(
            &self,
            req: Request<tonic::Streaming<MarketMakerQuote>>,
        ) -> std::result::Result<Response<Self::StreamQuotesStream>, Status> {
            let mut inbound = req.into_inner();
            let (tx, rx) = tokio::sync::mpsc::channel(8);

            tokio::spawn(async move {
                while let Ok(Some(quote)) = inbound.message().await {
                    let update_type =
                        if quote.bid_levels.is_empty() && quote.ask_levels.is_empty() {
                            UpdateType::Rejected
                        } else {
                            UpdateType::New
                        };
                    let update = QuoteUpdate {
                        update_type: update_type as i32,
                        status_message: None,
                    };
                    if tx.send(Ok(update)).await.is_err() {
                        break;
                    }
                }
            });

            Ok(Response::new(ReceiverStream::new(rx)))
        }

        type StreamSwapStream = ReceiverStream<std::result::Result<SwapUpdate, Status>>;

        async fn stream_swap(
            &self,
            _req: Request<tonic::Streaming<MarketMakerSwap>>,
        ) -> std::result::Result<Response<Self::StreamSwapStream>, Status> {
            unimplemented!()
        }

        async fn get_quotes(
            &self,
            req: Request<GetQuotesRequest>,
        ) -> std::result::Result<Response<GetQuotesResponse>, Status> {
            let inner = req.into_inner();
            // Echo back a single fake quote for the requested pair
            let quote = MarketMakerQuote {
                timestamp: 1_000_000,
                sequence_number: 1,
                quote_expiry_time: 30,
                maker_id: "test-maker".to_string(),
                maker_address: "11111111111111111111111111111111".to_string(),
                lot_size_base: 1000,
                cluster: Cluster::Mainnet as i32,
                token_pair: inner.token_pair,
                bid_levels: vec![PriceLevel {
                    volume: 1_000_000_000,
                    price: 150_000_000,
                }],
                ask_levels: vec![PriceLevel {
                    volume: 1_000_000_000,
                    price: 151_000_000,
                }],
            };
            Ok(Response::new(GetQuotesResponse {
                quotes: vec![quote],
            }))
        }
    }

    /// Spin up a mock gRPC server on a random port and return the client.
    async fn setup_test_client() -> MarketMakerClient {
        setup_test_client_with_auth(None).await
    }

    /// Same, but with an optional auth token on the client config.
    async fn setup_test_client_with_auth(auth_token: Option<&str>) -> MarketMakerClient {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(MarketMakerIngestionServiceServer::new(MockService))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        // Give the server a moment to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut config = ClientConfig::new(format!("http://{}", addr));
        if let Some(auth_token) = auth_token {
            config = config.with_auth_token(auth_token);
        }

        MarketMakerClient::connect_with_config(config)
            .await
            .expect("failed to connect to mock server")
    }

    #[tokio::test]
    async fn test_get_all_orderbooks_sends_api_key() {
        let mut client = setup_test_client_with_auth(Some("test-token")).await;
        client
            .get_all_orderbooks(Some(Cluster::Mainnet))
            .await
            .expect("get_all_orderbooks should send the x-api-key header");

        let mut anonymous = setup_test_client_with_auth(None).await;
        assert!(
            anonymous
                .get_all_orderbooks(Some(Cluster::Mainnet))
                .await
                .is_err(),
            "server should reject a request with no x-api-key header"
        );
    }

    #[tokio::test]
    async fn test_get_quotes_returns_quotes() {
        let mut client = setup_test_client().await;
        let pair = TokenPair::sol_usdc();

        let resp = client
            .get_quotes(pair, "test-token".to_string())
            .await
            .expect("get_quotes should succeed");

        assert_eq!(resp.quotes.len(), 1);
        let quote = &resp.quotes[0];
        assert_eq!(quote.maker_id, "test-maker");
        assert_eq!(quote.bid_levels.len(), 1);
        assert_eq!(quote.ask_levels.len(), 1);
        assert_eq!(quote.bid_levels[0].price, 150_000_000);
        assert_eq!(quote.ask_levels[0].price, 151_000_000);
    }

    #[tokio::test]
    async fn test_get_quotes_preserves_token_pair() {
        let mut client = setup_test_client().await;
        const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        let pair = TokenPair::new(
            Token::new(
                "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
                8,
                "ETH",
                TOKEN_PROGRAM,
            ),
            Token::new(
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                6,
                "USDC",
                TOKEN_PROGRAM,
            ),
        );

        let resp = client
            .get_quotes(pair, "test-token".to_string())
            .await
            .expect("get_quotes should succeed");

        // The mock echoes back the requested token pair
        let returned_pair = &resp.quotes[0].token_pair;
        assert_eq!(returned_pair.base_token.symbol, "ETH");
        assert_eq!(returned_pair.quote_token.symbol, "USDC");
    }

    /// Round-trips a quote through the generic `StreamHandle`: send, receive,
    /// stats, health and close.
    #[tokio::test]
    async fn test_stream_quotes_round_trip() {
        let mut client = setup_test_client().await;
        let mut stream = client.start_streaming().await.expect("start_streaming");

        let quote = MarketMakerQuote::builder()
            .maker_id("test-maker")
            .sol_usdc_pair()
            .maker_address("11111111111111111111111111111111".to_string())
            .lot_size_base(1000)
            .bid_level(1_000_000_000, 150_000_000)
            .build()
            .expect("quote should build");

        stream.send(quote).await.expect("send should succeed");

        let update = stream
            .receive_update_timeout(Duration::from_secs(5))
            .await
            .expect("receive should not time out")
            .expect("server should send an update");
        assert_eq!(update.update_type, UpdateType::New as i32);

        let stats = stream.get_stats().await;
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.updates_received, 1);
        assert_eq!(stats.errors_encountered, 0);
        assert!(stream.is_healthy(Duration::from_secs(30)).await);

        stream.close().await;
        assert!(stream.is_closed());
        assert!(
            stream.send(MarketMakerQuote::default()).await.is_err(),
            "sending on a closed stream must fail"
        );
    }
}
