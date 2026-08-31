"""quipu-client — thin, typed Python client for the Quipu REST API.

Stdlib only. See ``docs/book/src/reference/python-client.md`` in the main
repository for the documented surface.
"""

from .client import QuipuClient, QuipuError
from .types import AskResult, EpisodeResult, KnotResult, RetractResult, SetResult

__version__ = "0.1.0"

__all__ = [
    "QuipuClient",
    "QuipuError",
    "AskResult",
    "EpisodeResult",
    "KnotResult",
    "RetractResult",
    "SetResult",
]
