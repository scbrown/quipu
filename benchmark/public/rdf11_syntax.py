#!/usr/bin/env python3
"""Score Quipu's RDF loaders against the W3C RDF 1.1 syntax suites (aegis-mhee08).

Four suites from the pinned W3C rdf-tests checkout: Turtle, N-Triples,
N-Quads and TriG. Every case listed in a manifest is scored; the report breaks
the totals down by the manifest's approval status instead of choosing which
cases count (the N-Triples manifest marks only 2 of its 70 cases Approved).

How a case is run, through the same surface a user loads data with:

  positive syntax   `quipu knot <file>` must exit 0.
  negative syntax   `quipu knot <file>` must fail with an RDF parse error. Any
                    other failure is not a rejection: it is reported as an error.
  evaluation        the file must load, and `quipu export --format ntriples`
                    must be isomorphic to the expected N-Triples. Terms compare
                    exactly as RDF 1.1 defines them (lexical form included;
                    language tags case-insensitively).

The expected files are read by an independent parser in this script, never by
Quipu: a loader that misreads both sides the same way would otherwise pass.

Two disclosed departures from a bare `knot`:

  * Turtle inputs get one leading `@base <retrieval IRI> .` line. `knot`
    resolves relative IRIs against the local file path; the suite defines the
    retrieval IRI as mf:assumedTestBase plus the file name. An in-document
    @base still overrides it, exactly as it overrides a retrieval IRI.
  * N-Quads and TriG are scored UNSUPPORTED: `knot` has no parser for either,
    and `quipu ingest --format nq` requires a caller-declared triple count,
    which would measure the declaration instead of the parser. Unsupported
    cases stay in the denominator and never pass.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import tempfile
from collections import Counter
from itertools import permutations
from pathlib import Path

from conformance_provenance import provenance

PINNED_SUITE_REVISION = "369a90d1a60c021b746df2e411da0ff36258a758"
XSD_STRING = "http://www.w3.org/2001/XMLSchema#string"
RDF_LANG_STRING = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"

SUITES = {
    "rdf-turtle": {"extension": "ttl", "loader": "knot", "prepend_base": True},
    "rdf-n-triples": {"extension": "nt", "loader": "knot", "prepend_base": False},
    "rdf-n-quads": {"extension": "nq", "loader": None,
                    "reason": "no N-Quads parser behind `knot`; `ingest --format nq` needs a declared count"},
    "rdf-trig": {"extension": "trig", "loader": None, "reason": "Quipu has no TriG parser"},
}

# A case starts at `<#name> rdf:type|a rdft:Test...` and runs to the next start;
# terminators vary (a line of its own, or a trailing " ." after the last value).
START = re.compile(r"(?m)^<#([^>]+)>\s+(?:rdf:type|a)\s+rdft:(Test\w+)")
ACTION = re.compile(r"mf:action\s+<([^>]+)>")
RESULT = re.compile(r"mf:result\s+<([^>]+)>")
APPROVAL = re.compile(r"rdft:approval\s+rdft:(\w+)")
BASE = re.compile(r"mf:assumedTestBase\s+<([^>]+)>")
ENTRIES = re.compile(r"(?ms)mf:entries\s*\((.*?)\)")


def kind_of(test_type: str) -> str:
    if test_type.endswith("PositiveSyntax"):
        return "positive"
    if test_type.endswith("NegativeSyntax"):
        return "negative"
    if test_type.endswith("NegativeEval"):
        return "negative-eval"
    if test_type.endswith("Eval"):
        return "eval"
    raise ValueError(f"unknown test type {test_type}")


def cases_of(manifest: str, suite: str) -> tuple[str, list[dict]]:
    # N-Triples and N-Quads admit no relative IRIs, so their manifests declare
    # no base; the published suite location stands in and is never consulted.
    base = BASE.search(manifest)
    base_iri = base.group(1) if base else f"https://w3c.github.io/rdf-tests/rdf/rdf11/{suite}/"
    cases = []
    starts = list(START.finditer(manifest))
    for index, start in enumerate(starts):
        name, test_type = start.group(1), start.group(2)
        end = starts[index + 1].start() if index + 1 < len(starts) else len(manifest)
        body = manifest[start.end():end]
        action = ACTION.search(body)
        if not action:
            raise ValueError(f"{name}: no mf:action")
        result = RESULT.search(body)
        approval = APPROVAL.search(body)
        cases.append({"name": name, "type": test_type, "kind": kind_of(test_type),
                      "action": action.group(1), "result": result.group(1) if result else None,
                      "approval": approval.group(1) if approval else "unmarked"})
    listed = ENTRIES.search(manifest)
    if not listed:
        raise ValueError("manifest has no mf:entries list")
    entries = re.findall(r"<#([^>]+)>", listed.group(1))
    found = {c["name"] for c in cases}
    if sorted(entries) != sorted(found):
        missing = sorted(set(entries) - found)[:5]
        extra = sorted(found - set(entries))[:5]
        raise ValueError(f"parsed cases do not match mf:entries (missing {missing}, extra {extra})")
    return base_iri, cases


# ---- an independent N-Triples reader for the comparison ----

_ECHAR = {"t": "\t", "b": "\b", "n": "\n", "r": "\r", "f": "\f", '"': '"', "'": "'", "\\": "\\"}
_TERM = re.compile(
    r'\s*(?:<(?P<iri>[^>]*)>|_:(?P<bnode>[^\s<"]+)'
    r'|"(?P<lit>(?:[^"\\]|\\.)*)"(?:@(?P<lang>[A-Za-z]+(?:-[A-Za-z0-9]+)*)|\^\^<(?P<dt>[^>]*)>)?)'
)


def _unescape(text: str, *, echars: bool) -> str:
    def replace(m: re.Match[str]) -> str:
        s = m.group(0)
        if s[1] in "uU":
            return chr(int(s[2:], 16))
        if echars and s[1] in _ECHAR:
            return _ECHAR[s[1]]
        raise ValueError(f"bad escape {s!r}")

    return re.sub(r"\\(?:u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8}|.)", replace, text)


def read_ntriples(text: str) -> list[tuple]:
    """Triples of terms: ("I", iri) | ("B", label) | ("L", lexical, datatype, lang)."""
    triples = []
    # Split on LF only: str.splitlines also splits on U+2028, U+0085 and friends.
    for number, line in enumerate(text.replace("\r\n", "\n").split("\n"), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        rest, terms = stripped, []
        for _ in range(3):
            m = _TERM.match(rest)
            if not m:
                raise ValueError(f"line {number}: unreadable term in {line!r}")
            if m.group("iri") is not None:
                terms.append(("I", _unescape(m.group("iri"), echars=False)))
            elif m.group("bnode") is not None:
                label = m.group("bnode")
                # A label may contain '.' but never end with one: a trailing dot
                # with no space before it is the statement terminator.
                terminator = label.endswith(".")
                terms.append(("B", label.rstrip(".")))
                rest = ("." if terminator else "") + rest[m.end():]
                continue
            else:
                lexical = _unescape(m.group("lit"), echars=True)
                lang = (m.group("lang") or "").lower()
                datatype = RDF_LANG_STRING if lang else _unescape(m.group("dt") or XSD_STRING, echars=False)
                terms.append(("L", lexical, datatype, lang))
            rest = rest[m.end():]
        if not re.fullmatch(r"\s*\.\s*(#.*)?", rest):
            raise ValueError(f"line {number}: expected '.' after three terms in {line!r}")
        triples.append(tuple(terms))
    return triples


def isomorphic(left: list[tuple], right: list[tuple]) -> bool:
    """Graph isomorphism modulo blank-node labels (RDF graphs are sets)."""
    a, b = set(left), set(right)
    if len(a) != len(b):
        return False

    def blanks(graph):
        return sorted({t[1] for triple in graph for t in triple if t[0] == "B"})

    ba, bb = blanks(a), blanks(b)
    if len(ba) != len(bb):
        return False
    if not ba:
        return a == b

    def signature(graph, rounds=3):
        colour = {n: "" for n in blanks(graph)}
        for _ in range(rounds):
            nxt = {}
            for n in colour:
                parts = []
                for s, p, o in graph:
                    for pos, term in (("s", s), ("o", o)):
                        if term == ("B", n):
                            other = o if pos == "s" else s
                            parts.append((pos, p, colour.get(other[1]) if other[0] == "B" else other))
                nxt[n] = repr(sorted(map(repr, parts)))
            colour = nxt
        return colour

    ca, cb = signature(a), signature(b)
    if Counter(ca.values()) != Counter(cb.values()):
        return False
    groups: dict[str, tuple[list[str], list[str]]] = {}
    for n, c in ca.items():
        groups.setdefault(c, ([], []))[0].append(n)
    for n, c in cb.items():
        groups[c][1].append(n)

    def relabel(graph, mapping):
        return {tuple(("B", mapping[t[1]]) if t[0] == "B" else t for t in triple) for triple in graph}

    ordered = list(groups.values())
    budget = [200_000]

    def search(index, mapping):
        if index == len(ordered):
            return relabel(a, mapping) == b
        src, dst = ordered[index]
        for perm in permutations(dst):
            budget[0] -= 1
            if budget[0] < 0:
                raise ValueError("isomorphism search exceeded its budget")
            if search(index + 1, {**mapping, **dict(zip(src, perm))}):
                return True
        return False

    return search(0, {})


# ---- running one case ----

def load(quipu: Path, source: Path, database: Path) -> subprocess.CompletedProcess:
    return subprocess.run([str(quipu), "knot", str(source), "--db", str(database)],
                          capture_output=True, text=True, timeout=120)


def run_case(case: dict, suite: str, directory: Path, base: str, quipu: Path, work: Path) -> dict:
    config = SUITES[suite]
    outcome = {"test": f"{suite}/{case['name']}", "kind": case["kind"], "approval": case["approval"]}
    if config["loader"] is None:
        return {**outcome, "observed": "unsupported", "passed": False, "diagnostic": config["reason"]}
    source = directory / case["action"]
    # BYTES, never text: universal-newline decoding turns a raw CR inside a
    # literal into LF, which then fails a correct parser (measured).
    data = source.read_bytes()
    staged = work / f"case.{config['extension']}"
    if config["prepend_base"]:
        data = f"@base <{base}{case['action']}> .\n".encode() + data
    staged.write_bytes(data)
    database = work / "case.db"
    for leftover in work.glob("case.db*"):
        leftover.unlink()
    loaded = load(quipu, staged, database)
    parse_error = loaded.returncode != 0 and "RDF parse error" in loaded.stderr
    if loaded.returncode == 0:
        observed = "accept"
    elif parse_error:
        observed = "reject"
    else:
        observed = "error"
    diagnostic = loaded.stderr.strip()[:500] if loaded.returncode else ""

    if case["kind"] == "positive":
        return {**outcome, "observed": observed, "passed": observed == "accept", "diagnostic": diagnostic}
    if case["kind"] in ("negative", "negative-eval"):
        return {**outcome, "observed": observed, "passed": observed == "reject", "diagnostic": diagnostic}
    if observed != "accept":
        return {**outcome, "observed": observed, "passed": False, "diagnostic": diagnostic}
    exported = subprocess.run([str(quipu), "export", "--format", "ntriples", "--db", str(database)],
                              capture_output=True, timeout=120)
    if exported.returncode:
        return {**outcome, "observed": "error", "passed": False,
                "diagnostic": f"export failed: {exported.stderr.decode(errors='replace').strip()[:300]}"}
    try:
        actual = read_ntriples(exported.stdout.decode("utf-8"))
        expected = read_ntriples((directory / case["result"]).read_bytes().decode("utf-8"))
        same = isomorphic(actual, expected)
    except ValueError as error:
        return {**outcome, "observed": "error", "passed": False, "diagnostic": str(error)}
    diff = ""
    if not same:
        only_actual = sorted(map(repr, set(actual) - set(expected)))[:3]
        only_expected = sorted(map(repr, set(expected) - set(actual)))[:3]
        diff = f"graph differs; only in Quipu {only_actual}; only in expected {only_expected}"
    return {**outcome, "observed": "equal" if same else "different", "passed": same, "diagnostic": diff}


def git_output(cwd: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, text=True,
                          capture_output=True).stdout.strip()


def summarise(results: list[dict]) -> dict:
    def totals(rows):
        passed = sum(r["passed"] for r in rows)
        return {"passed": passed, "failed": len(rows) - passed, "cases": len(rows),
                "unsupported": sum(r["observed"] == "unsupported" for r in rows)}

    by_suite = {}
    for suite in SUITES:
        rows = [r for r in results if r["test"].startswith(suite + "/")]
        by_suite[suite] = {**totals(rows),
                           "by_approval": {a: totals([r for r in rows if r["approval"] == a])
                                           for a in sorted({r["approval"] for r in rows})},
                           "by_kind": {k: totals([r for r in rows if r["kind"] == k])
                                       for k in sorted({r["kind"] for r in rows})}}
    return {"all": totals(results), "suites": by_suite}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--suite", required=True, type=Path, help="a W3C rdf-tests checkout")
    parser.add_argument("--quipu", default="quipu", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--allow-unpinned-suite", action="store_true")
    args = parser.parse_args()

    revision = git_output(args.suite, "rev-parse", "HEAD")
    dirty = git_output(args.suite, "status", "--porcelain")
    if not args.allow_unpinned_suite and (revision != PINNED_SUITE_REVISION or dirty):
        parser.error(f"suite must be clean at {PINNED_SUITE_REVISION}; got {revision}")
    quipu = Path(shutil.which(str(args.quipu)) or args.quipu)
    version = subprocess.run([str(quipu), "--version"], check=True, text=True,
                             capture_output=True).stdout.strip()
    quipu_root = Path(__file__).resolve().parents[2]

    results = []
    with tempfile.TemporaryDirectory(prefix="quipu-rdf11-syntax-") as tmp:
        work = Path(tmp)
        for suite in SUITES:
            directory = args.suite / "rdf" / "rdf11" / suite
            base, cases = cases_of((directory / "manifest.ttl").read_text(), suite)
            if not cases:
                parser.error(f"{suite}: manifest produced zero cases")
            for case in cases:
                results.append(run_case(case, suite, directory, base, quipu, work))

    report = {
        "benchmark": "W3C RDF 1.1 syntax suites (Turtle, N-Triples, N-Quads, TriG)",
        "suite_revision": revision,
        "quipu_revision": git_output(quipu_root, "rev-parse", "HEAD"),
        "quipu_version": version,
        "scope": ("every manifest case; loading through `quipu knot`, evaluation compared after "
                  "`quipu export --format ntriples`; N-Quads and TriG unsupported"),
        "departures": [
            "Turtle inputs get a leading @base line naming the suite's retrieval IRI",
            "expected N-Triples are read by this script's own parser, not by Quipu",
        ],
        "totals": summarise(results),
        "results": results,
    }
    report["generated_at"], report["generated_by"] = provenance()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({s: {k: v[k] for k in ("passed", "cases", "unsupported")}
                      for s, v in report["totals"]["suites"].items()}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
