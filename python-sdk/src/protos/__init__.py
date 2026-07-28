"""Generated protobuf code for Jupiter RFQ gRPC service.

The two generated modules ``market_maker_pb2`` and ``market_maker_pb2_grpc``
are produced from ``../../protos/market_maker.proto`` at build time and are
intentionally **not** committed to the repository. They are regenerated:

* automatically by ``pip install`` (via ``setup.py`` ``build_py`` hook), or
* manually via ``python scripts/generate_protos.py``.

This package exists to turn a missing-stubs ``ModuleNotFoundError`` into an
actionable one. Import the names themselves from the generated modules, e.g.
``from protos.market_maker_pb2 import MarketMakerQuote`` — importing either
submodule runs this guard first.
"""

try:
    from . import market_maker_pb2, market_maker_pb2_grpc  # noqa: F401
except ImportError as exc:  # pragma: no cover - install-time error
    # Absent submodules surface as ImportError ("cannot import name ... from
    # partially initialized module"), not ModuleNotFoundError.
    raise ModuleNotFoundError(
        "Generated protobuf stubs are missing. Run "
        "`python scripts/generate_protos.py` from the python-sdk/ directory, "
        "or reinstall with `pip install .`."
    ) from exc
