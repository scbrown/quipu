"""Typed envelopes for the obvious Quipu response shapes.

Each dataclass keeps the full decoded body in ``raw`` — the typed fields are
the documented contract, ``raw`` is the escape hatch when the server says
more than the contract promises. Shapes mirror
``docs/book/src/reference/rest-api.md``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class KnotResult:
    """``POST /knot`` — ``{"tx_id": 1, "count": 2, "conforms": true}``."""

    tx_id: int
    count: int
    conforms: bool
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_body(cls, body: dict[str, Any]) -> "KnotResult":
        return cls(
            tx_id=body.get("tx_id", 0),
            count=body.get("count", 0),
            conforms=body.get("conforms", True),
            raw=body,
        )


@dataclass(frozen=True)
class EpisodeResult:
    """``POST /episode``.

    Branch on ``outcome`` (``created`` / ``updated`` / ``unchanged``), never
    on ``count`` — ``unchanged`` is a successful idempotent retry that wrote
    nothing and needed to write nothing.
    """

    outcome: str
    count: int
    tx_id: int
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_body(cls, body: dict[str, Any]) -> "EpisodeResult":
        return cls(
            outcome=body.get("outcome", ""),
            count=body.get("count", 0),
            tx_id=body.get("tx_id", 0),
            raw=body,
        )


@dataclass(frozen=True)
class SetResult:
    """``POST /set`` — single-call supersede.

    ``tx_id == 0`` with nothing retracted or asserted is the documented
    idempotent no-op: the value was already the sole current one.
    """

    tx_id: int
    retracted: int
    asserted: int
    entity: str
    predicate: str
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_body(cls, body: dict[str, Any]) -> "SetResult":
        return cls(
            tx_id=body.get("tx_id", 0),
            retracted=body.get("retracted", 0),
            asserted=body.get("asserted", 0),
            entity=body.get("entity", ""),
            predicate=body.get("predicate", ""),
            raw=body,
        )


@dataclass(frozen=True)
class RetractResult:
    """``POST /retract`` — ``{"retracted": 0}`` is quiet idempotence for a
    correctly shaped value; a bare string that cannot match is a 400 and
    raises ``QuipuError`` instead."""

    retracted: int
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_body(cls, body: dict[str, Any]) -> "RetractResult":
        return cls(retracted=body.get("retracted", 0), raw=body)


@dataclass(frozen=True)
class AskResult:
    """``POST /ask`` — a curated named query's resolved run."""

    query: str
    sparql: str
    columns: list[str]
    rows: list[Any]
    count: int
    raw: dict[str, Any] = field(repr=False)

    @classmethod
    def from_body(cls, body: dict[str, Any]) -> "AskResult":
        return cls(
            query=body.get("query", ""),
            sparql=body.get("sparql", ""),
            columns=body.get("columns", []),
            rows=body.get("rows", []),
            count=body.get("count", 0),
            raw=body,
        )
