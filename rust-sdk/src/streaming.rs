//! Bidirectional gRPC streaming for quotes and swaps.
//!
//! The client sends messages to the ingestion service and receives real-time
//! updates back over the same stream. Quotes and swaps share one handle type;
//! see the [`QuoteStreamHandle`] and [`SwapStreamHandle`] aliases.

use crate::error::{MarketMakerError, Result};
use crate::types::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tonic::Streaming;

/// Handle for a bidirectional gRPC stream: `Out` is sent to the server,
/// `In` is received from it.
pub struct StreamHandle<Out, In> {
    sender: mpsc::UnboundedSender<Out>,
    /// Direct gRPC stream of incoming updates from the server
    pub update_receiver: Streaming<In>,
    stats: Arc<Mutex<ConnectionStats>>,
    is_closed: Arc<Mutex<bool>>,
}

/// Stream of outbound [`MarketMakerQuote`]s and inbound [`QuoteUpdate`]s
pub type QuoteStreamHandle = StreamHandle<MarketMakerQuote, QuoteUpdate>;

/// Stream of outbound [`MarketMakerSwap`]s and inbound [`SwapUpdate`]s
pub type SwapStreamHandle = StreamHandle<MarketMakerSwap, SwapUpdate>;

impl<Out, In> StreamHandle<Out, In> {
    pub(crate) fn new(sender: mpsc::UnboundedSender<Out>, update_receiver: Streaming<In>) -> Self {
        Self {
            sender,
            update_receiver,
            stats: Arc::new(Mutex::new(ConnectionStats::new())),
            is_closed: Arc::new(Mutex::new(false)),
        }
    }

    /// Send a message to the gRPC server
    pub async fn send(&self, msg: Out) -> Result<()> {
        if *self.is_closed.lock().await {
            return Err(MarketMakerError::streaming("Stream has been closed"));
        }

        self.sender
            .send(msg)
            .map_err(|_| MarketMakerError::streaming("Failed to send - gRPC stream closed"))?;

        self.stats.lock().await.message_sent();
        Ok(())
    }

    /// Receive the next update from the gRPC server.
    ///
    /// `Ok(None)` means the server closed the stream.
    pub async fn receive_update(&mut self) -> Result<Option<In>> {
        if *self.is_closed.lock().await {
            return Ok(None);
        }

        match self.update_receiver.message().await {
            Ok(Some(update)) => {
                self.stats.lock().await.update_received();
                Ok(Some(update))
            }
            Ok(None) => {
                // Stream ended normally - mark as closed
                *self.is_closed.lock().await = true;
                Ok(None)
            }
            Err(e) => {
                self.stats.lock().await.error_encountered();
                Err(MarketMakerError::Grpc(e))
            }
        }
    }

    /// Receive the next update, giving up after `timeout`
    pub async fn receive_update_timeout(&mut self, timeout: Duration) -> Result<Option<In>> {
        tokio::time::timeout(timeout, self.receive_update())
            .await
            .map_err(|_| MarketMakerError::timeout("Timed out waiting for update"))?
    }

    /// Whether the stream has seen traffic within `inactivity_timeout`
    pub async fn is_healthy(&self, inactivity_timeout: Duration) -> bool {
        let stats = self.stats.lock().await;
        match stats.time_since_last_activity() {
            Some(idle) => idle <= inactivity_timeout,
            // No activity yet, but the connection is new
            None => stats.connected_at.elapsed() < inactivity_timeout,
        }
    }

    /// Close the gRPC stream gracefully
    pub async fn close(&mut self) {
        tracing::info!("Initiating graceful stream shutdown");

        // Mark as closed first to prevent new operations
        *self.is_closed.lock().await = true;

        // Close the outbound half by dropping the sender
        drop(std::mem::replace(
            &mut self.sender,
            mpsc::unbounded_channel().0,
        ));

        // Give in-flight frames a brief moment to leave
        tokio::time::sleep(Duration::from_millis(100)).await;

        tracing::info!("Stream shutdown completed");
    }

    /// Close the stream, giving up after `timeout`
    pub async fn close_with_timeout(&mut self, timeout: Duration) -> Result<()> {
        if tokio::time::timeout(timeout, self.close()).await.is_err() {
            tracing::warn!("Stream close timed out after {:?}", timeout);
            // Force close by marking as closed
            *self.is_closed.lock().await = true;
            return Err(MarketMakerError::timeout("Stream close operation timed out"));
        }
        Ok(())
    }

    /// Whether the stream is closed
    pub async fn is_closed(&self) -> bool {
        *self.is_closed.lock().await || self.sender.is_closed()
    }

    /// Snapshot of the connection statistics
    pub async fn get_stats(&self) -> ConnectionStats {
        self.stats.lock().await.clone()
    }
}

/// Connection statistics for monitoring stream health
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub messages_sent: u64,
    pub updates_received: u64,
    pub errors_encountered: u64,
    pub connected_at: Instant,
    pub last_activity: Option<Instant>,
}

impl ConnectionStats {
    pub fn new() -> Self {
        Self {
            messages_sent: 0,
            updates_received: 0,
            errors_encountered: 0,
            connected_at: Instant::now(),
            last_activity: None,
        }
    }

    fn message_sent(&mut self) {
        self.messages_sent += 1;
        self.last_activity = Some(Instant::now());
    }

    fn update_received(&mut self) {
        self.updates_received += 1;
        self.last_activity = Some(Instant::now());
    }

    fn error_encountered(&mut self) {
        self.errors_encountered += 1;
    }

    /// Time since the last successful send or receive
    pub fn time_since_last_activity(&self) -> Option<Duration> {
        self.last_activity.map(|instant| instant.elapsed())
    }
}

impl Default for ConnectionStats {
    fn default() -> Self {
        Self::new()
    }
}
