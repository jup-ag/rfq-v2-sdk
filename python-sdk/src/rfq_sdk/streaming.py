"""Streaming functionality for real-time quote and swap updates.

Provides bidirectional gRPC streaming between the market maker client and the
ingestion service. The client can send quotes / swaps to the server and receive
real-time updates.
"""

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import AsyncIterator, Optional, Union

import grpc

from protos.market_maker_pb2 import (
    MarketMakerQuote,
    MarketMakerSwap,
    QuoteUpdate,
    SwapMessageType,
    SwapUpdate,
    UpdateType,
)

from .error import GrpcError, StreamingError, TimeoutError

logger = logging.getLogger(__name__)

OutboundMessage = Union[MarketMakerQuote, MarketMakerSwap]
InboundUpdate = Union[QuoteUpdate, SwapUpdate]


# --- Stream configuration --------------------------------------------------

@dataclass
class StreamConfig:
    """Configuration for streaming behavior."""

    send_buffer_size: int = 1000
    inactivity_timeout: timedelta = field(default_factory=lambda: timedelta(seconds=120))

    def with_send_buffer_size(self, size: int) -> "StreamConfig":
        self.send_buffer_size = size
        return self

    def with_inactivity_timeout(self, timeout: timedelta) -> "StreamConfig":
        self.inactivity_timeout = timeout
        return self


# --- Connection statistics -------------------------------------------------

@dataclass
class ConnectionStats:
    """Generic connection statistics for monitoring stream health.

    ``connected_at`` and ``last_activity`` are :class:`datetime` instances.
    """

    messages_sent: int = 0
    updates_received: int = 0
    errors_encountered: int = 0
    connected_at: datetime = field(default_factory=datetime.now)
    last_activity: Optional[datetime] = None

    def activity(self) -> None:
        self.last_activity = datetime.now()

    def message_sent(self) -> None:
        self.messages_sent += 1
        self.activity()

    def update_received(self) -> None:
        self.updates_received += 1
        self.activity()

    def error_encountered(self) -> None:
        self.errors_encountered += 1

    def time_since_last_activity(self) -> Optional[timedelta]:
        if self.last_activity is None:
            return None
        return datetime.now() - self.last_activity

    def elapsed(self) -> timedelta:
        """Time elapsed since the connection was established."""
        return datetime.now() - self.connected_at


# --- Stream handle ---------------------------------------------------------

class StreamHandle:
    """Handle for managing a bidirectional gRPC stream.

    Used for both quote streams and swap streams; the only difference is the
    message type flowing in each direction.
    """

    def __init__(
        self,
        queue: asyncio.Queue,
        update_stream: "grpc.aio.StreamStreamCall",
        config: Optional[StreamConfig] = None,
        kind: str = "stream",
    ):
        self._queue = queue
        self._update_stream = update_stream
        self._config = config or StreamConfig()
        self._kind = kind
        self._is_closed = False
        self._stats = ConnectionStats()
        self._stats_lock = asyncio.Lock()

    async def send(self, msg: OutboundMessage) -> None:
        """Send a message to the gRPC server.

        Raises :class:`StreamingError` if the stream is closed.
        """
        if self._is_closed:
            raise StreamingError("Stream has been closed")
        try:
            await self._queue.put(msg)
        except Exception as exc:  # pragma: no cover - asyncio.Queue does not raise
            async with self._stats_lock:
                self._stats.error_encountered()
            raise StreamingError(f"Failed to send message: {exc}") from exc

        async with self._stats_lock:
            self._stats.message_sent()

    async def receive_update(self) -> Optional[InboundUpdate]:
        """Receive the next update from the gRPC server.

        Returns ``None`` when the stream ends.
        """
        if self._is_closed:
            return None
        try:
            update = await self._update_stream.read()
        except asyncio.CancelledError:
            self._is_closed = True
            return None
        except grpc.RpcError as rpc_err:
            async with self._stats_lock:
                self._stats.error_encountered()
            raise GrpcError(rpc_err) from rpc_err

        # gRPC aio uses an end-of-stream sentinel; both ``None`` and
        # ``grpc.aio.EOF`` indicate the stream is finished.
        if update is None or update is grpc.aio.EOF:
            self._is_closed = True
            return None

        async with self._stats_lock:
            self._stats.update_received()
        return update

    async def receive_update_timeout(self, timeout_secs: float) -> Optional[InboundUpdate]:
        """Receive an update with a timeout (in seconds)."""
        try:
            return await asyncio.wait_for(self.receive_update(), timeout=timeout_secs)
        except asyncio.TimeoutError as exc:
            raise TimeoutError("Timed out waiting for update") from exc

    async def close(self) -> None:
        """Close the gRPC stream gracefully.

        Never blocks: signalling the outbound iterator and cancelling the
        inbound call are both non-blocking, so there is nothing to time out on.
        """
        if self._is_closed:
            return
        logger.info("Initiating graceful %s shutdown", self._kind)
        self._is_closed = True

        # Signal the outbound iterator to stop
        try:
            self._queue.put_nowait(None)
        except asyncio.QueueFull:
            pass

        # Cancel the inbound stream
        try:
            self._update_stream.cancel()
        except Exception:  # noqa: BLE001 - best effort cancel
            pass

        logger.info("%s shutdown completed", self._kind.capitalize())

    async def is_closed(self) -> bool:
        """Check if the stream is closed."""
        return self._is_closed

    async def get_stats(self) -> ConnectionStats:
        """Return a snapshot of the connection statistics."""
        async with self._stats_lock:
            return ConnectionStats(
                messages_sent=self._stats.messages_sent,
                updates_received=self._stats.updates_received,
                errors_encountered=self._stats.errors_encountered,
                connected_at=self._stats.connected_at,
                last_activity=self._stats.last_activity,
            )

    async def is_healthy(self, config: Optional[StreamConfig] = None) -> bool:
        """Return ``True`` while the stream has had recent activity.

        Uses :attr:`StreamConfig.inactivity_timeout`.
        """
        cfg = config or self._config
        async with self._stats_lock:
            since = self._stats.time_since_last_activity()
            if since is not None:
                return since <= cfg.inactivity_timeout
            return self._stats.elapsed() < cfg.inactivity_timeout

    async def updates(self) -> AsyncIterator[InboundUpdate]:
        """Async generator yielding updates until the stream ends."""
        while not self._is_closed:
            update = await self.receive_update()
            if update is None:
                break
            yield update


# Aliases kept for readable type hints at call sites.
QuoteStreamHandle = StreamHandle
SwapStreamHandle = StreamHandle


# --- Quote update helpers (mirrors `update_helpers` mod) -------------------

class update_helpers:  # noqa: N801 - mirrors Rust module name
    """Helpers for inspecting :class:`QuoteUpdate` messages.

    Implemented as a class with only static methods so callers can use it as
    ``streaming.update_helpers.is_new_quote(u)`` — mirroring Rust's
    ``streaming::update_helpers::is_new_quote(u)``.
    """

    @staticmethod
    def is_heartbeat(update: QuoteUpdate) -> bool:
        return update.update_type == UpdateType.UPDATE_TYPE_UNSPECIFIED

    @staticmethod
    def is_new_quote(update: QuoteUpdate) -> bool:
        return update.update_type == UpdateType.UPDATE_TYPE_NEW

    @staticmethod
    def is_updated_quote(update: QuoteUpdate) -> bool:
        return update.update_type == UpdateType.UPDATE_TYPE_UPDATED

    @staticmethod
    def is_expired_quote(update: QuoteUpdate) -> bool:
        return update.update_type == UpdateType.UPDATE_TYPE_EXPIRED

    @staticmethod
    def is_rejected_quote(update: QuoteUpdate) -> bool:
        return update.update_type == UpdateType.UPDATE_TYPE_REJECTED

    @staticmethod
    def get_status_message(update: QuoteUpdate) -> Optional[str]:
        msg = getattr(update, "status_message", "")
        return msg if msg else None

    @staticmethod
    def update_type_description(update: QuoteUpdate) -> str:
        if update.update_type == UpdateType.UPDATE_TYPE_NEW:
            return "New Quote"
        if update.update_type == UpdateType.UPDATE_TYPE_UPDATED:
            return "Updated Quote"
        if update.update_type == UpdateType.UPDATE_TYPE_EXPIRED:
            return "Expired Quote"
        if update.update_type == UpdateType.UPDATE_TYPE_REJECTED:
            return "Rejected Quote"
        return "System Message"


# --- Swap update helpers (mirrors `swap_update_helpers` mod) ---------------

class swap_update_helpers:  # noqa: N801 - mirrors Rust module name
    """Helpers for inspecting :class:`SwapUpdate` messages.

    Mirrors Rust's ``streaming::swap_update_helpers`` module.
    """

    @staticmethod
    def is_pong(update: SwapUpdate) -> bool:
        return update.message_type == SwapMessageType.SWAP_MESSAGE_TYPE_PONG

    @staticmethod
    def is_connection_ready(update: SwapUpdate) -> bool:
        return update.message_type == SwapMessageType.SWAP_MESSAGE_TYPE_CONNECTION_READY

    @staticmethod
    def is_swap_available(update: SwapUpdate) -> bool:
        return update.message_type == SwapMessageType.SWAP_MESSAGE_TYPE_SWAP_AVAILABLE

    @staticmethod
    def is_transaction_confirmed(update: SwapUpdate) -> bool:
        return update.message_type == SwapMessageType.SWAP_MESSAGE_TYPE_TRANSACTION_CONFIRMED

    @staticmethod
    def is_error(update: SwapUpdate) -> bool:
        return update.message_type == SwapMessageType.SWAP_MESSAGE_TYPE_ERROR

    @staticmethod
    def get_swap_uuid(update: SwapUpdate) -> Optional[str]:
        uuid = getattr(update, "swap_uuid", "")
        return uuid if uuid else None

    @staticmethod
    def get_unsigned_transaction(update: SwapUpdate) -> Optional[str]:
        tx = getattr(update, "unsigned_transaction", "")
        return tx if tx else None

    @staticmethod
    def get_transaction_signature(update: SwapUpdate) -> Optional[str]:
        sig = getattr(update, "transaction_signature", "")
        return sig if sig else None

    @staticmethod
    def get_status_message(update: SwapUpdate) -> Optional[str]:
        msg = getattr(update, "status_message", "")
        return msg if msg else None

    @staticmethod
    def update_type_description(update: SwapUpdate) -> str:
        mapping = {
            SwapMessageType.SWAP_MESSAGE_TYPE_PING: "Ping",
            SwapMessageType.SWAP_MESSAGE_TYPE_PONG: "Pong",
            SwapMessageType.SWAP_MESSAGE_TYPE_CONNECTION_READY: "Connection Ready",
            SwapMessageType.SWAP_MESSAGE_TYPE_SWAP_AVAILABLE: "Swap Available",
            SwapMessageType.SWAP_MESSAGE_TYPE_SWAP_SUBMIT: "Swap Submit",
            SwapMessageType.SWAP_MESSAGE_TYPE_TRANSACTION_CONFIRMED: "Transaction Confirmed",
            SwapMessageType.SWAP_MESSAGE_TYPE_ERROR: "Error",
        }
        return mapping.get(update.message_type, "Unknown Message Type")

    @staticmethod
    def extract_swap_details(update: SwapUpdate):  # -> Optional[Tuple[str, str]]
        """Return ``(swap_uuid, unsigned_transaction)`` for an available swap."""
        if not swap_update_helpers.is_swap_available(update):
            return None
        uuid = swap_update_helpers.get_swap_uuid(update)
        tx = swap_update_helpers.get_unsigned_transaction(update)
        if uuid and tx:
            return (uuid, tx)
        return None

    @staticmethod
    def extract_confirmation_details(update: SwapUpdate):  # -> Optional[Tuple[str, str]]
        """Return ``(swap_uuid, transaction_signature)`` for a confirmation."""
        if not swap_update_helpers.is_transaction_confirmed(update):
            return None
        uuid = swap_update_helpers.get_swap_uuid(update)
        sig = swap_update_helpers.get_transaction_signature(update)
        if uuid and sig:
            return (uuid, sig)
        return None


__all__ = [
    "ConnectionStats",
    "QuoteStreamHandle",
    "StreamConfig",
    "StreamHandle",
    "SwapStreamHandle",
    "swap_update_helpers",
    "update_helpers",
]
