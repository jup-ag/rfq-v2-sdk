"""Jupiter RFQv2 SDK for Python.

This SDK provides a Python interface for the RFQv2 Ingestion Service.

Key entry points:

* :class:`MarketMakerClient` — async gRPC client (unary RPCs + streaming).
* :class:`MarketMakerQuoteBuilder` — fluent builder for quotes.
* :mod:`rfq_sdk.streaming` — :class:`StreamHandle` (aliased as
  :class:`QuoteStreamHandle` / :class:`SwapStreamHandle`),
  :class:`update_helpers`, :class:`swap_update_helpers`.
* :mod:`rfq_sdk.error` — :class:`MarketMakerError` and subclasses.

Each submodule defines its own ``__all__``; the names below are the
convenience re-exports available directly on ``rfq_sdk``.
"""

__version__ = "0.1.0"
VERSION: str = __version__

# --- Submodules (re-exported as `rfq_sdk.<name>`) -------------------------

from . import builders, error, streaming, types

# --- Client ---------------------------------------------------------------

from .client import MarketMakerClient

# --- Errors ---------------------------------------------------------------

from .error import (
    ConfigurationError,
    ConnectionError,
    GrpcError,
    MarketMakerError,
    OtherError,
    SerializationError,
    StreamingError,
    TimeoutError,
    ValidationError,
)

# --- Constants, configuration, helpers & protobuf types -------------------

from .types import (
    DEFAULT_CHANNEL_BUFFER_SIZE,
    DEFAULT_ENDPOINT,
    DEFAULT_TIMEOUT_SECS,
    ClientConfig,
    Cluster,
    GetAllOrderbooksRequest,
    GetAllOrderbooksResponse,
    GetQuotesRequest,
    GetQuotesResponse,
    MarketMakerQuote,
    MarketMakerSwap,
    Orderbook,
    PriceLevel,
    QuoteHelper,
    QuoteResponse,
    QuoteUpdate,
    SequenceNumberRequest,
    SequenceNumberResponse,
    SwapMessageType,
    SwapUpdate,
    Token,
    TokenPair,
    TokenPairHelper,
    UpdateType,
)

# --- Builders -------------------------------------------------------------

from .builders import DEFAULT_QUOTE_EXPIRY_SECS, MarketMakerQuoteBuilder

# --- Streaming ------------------------------------------------------------

from .streaming import (
    ConnectionStats,
    QuoteStreamHandle,
    StreamConfig,
    StreamHandle,
    SwapStreamHandle,
    swap_update_helpers,
    update_helpers,
)

# Composed from each submodule's own ``__all__`` rather than hand-listed, so the
# export list has a single source of truth.
__all__ = [
    "__version__",
    "VERSION",
    "MarketMakerClient",
    "builders",
    "error",
    "streaming",
    "types",
    *error.__all__,
    *types.__all__,
    *builders.__all__,
    *streaming.__all__,
]
