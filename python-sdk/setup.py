"""Proto-generation build hooks for the Jupiter RFQv2 SDK.

All packaging metadata lives in ``pyproject.toml``; this file exists only to
regenerate the protobuf stubs at build time.

The Python protobuf stubs (``src/protos/market_maker_pb2*.py``) are not
checked into git — see ``.gitignore``. They are produced from
``../protos/market_maker.proto`` and need to be generated before importing
the SDK.

Generation strategy:

* Editable installs (``pip install -e .``) and any build that can see the
  sibling ``../protos/`` directory: the :class:`BuildPy` / :class:`Develop`
  commands run :func:`generate_protos.generate` automatically.
* Isolated PEP 517 builds (``pip install .`` from a fresh checkout): the
  parent ``../protos/`` directory is **not** copied into the build sandbox,
  so we skip generation and rely on the user running
  ``python scripts/generate_protos.py`` first.

Run the generator manually any time::

    python scripts/generate_protos.py
"""

import sys
from pathlib import Path

import setuptools
from setuptools import setup
from setuptools.command.build_py import build_py
from setuptools.command.develop import develop

# PEP 621 metadata (``[project]`` in pyproject.toml) needs setuptools >= 61.
# Older versions silently build a nameless, package-less 0.0.0 distribution
# instead of failing, so refuse rather than mis-install.
if tuple(int(p) for p in setuptools.__version__.split(".")[:2]) < (61, 0):
    raise SystemExit(
        f"setuptools>=61.0 is required to build this package "
        f"(found {setuptools.__version__}); run `pip install -U setuptools`."
    )


def _try_generate_protos() -> None:
    """Regenerate stubs in place when the proto file is reachable.

    For a clone of the repo (where ``../protos/market_maker.proto`` is a
    sibling directory) this regenerates the stubs every build. For an
    isolated PEP 517 build the parent directory is **not** copied into the
    build sandbox — in that case we leave whatever stubs are already present
    in ``src/protos/`` alone and let the import-time check in
    ``protos/__init__.py`` give the user a clear error.
    """
    here = Path(__file__).resolve().parent
    proto_file = here.parent / "protos" / "market_maker.proto"
    stubs_present = (here / "src" / "protos" / "market_maker_pb2.py").exists()

    if not proto_file.is_file():
        if not stubs_present:
            sys.stderr.write(
                "\n[setup.py] WARNING: protobuf stubs are not generated and the "
                "source proto file is not reachable from this build sandbox.\n"
                "           Run from a fresh clone:\n"
                "             python scripts/generate_protos.py\n"
                "             pip install .\n\n"
            )
        return

    sys.path.insert(0, str(here / "scripts"))
    try:
        from generate_protos import generate  # type: ignore[import-not-found]

        generate(here)
    finally:
        if str(here / "scripts") in sys.path:
            sys.path.remove(str(here / "scripts"))


class BuildPy(build_py):
    """``build_py`` step that regenerates the proto stubs first."""

    def run(self):
        _try_generate_protos()
        super().run()


class Develop(develop):
    """``develop`` (editable install) — also generate protos in-place."""

    def run(self):
        _try_generate_protos()
        super().run()


setup(cmdclass={"build_py": BuildPy, "develop": Develop})
