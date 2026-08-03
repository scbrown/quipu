#!/usr/bin/env python3
"""Walk sibling repos and emit Turtle against the code-entities vocabulary.

Produces CodeModule / CodeSymbol / Document / Section instances for the
`shapes/code-entities.ttl` shapes, so the ingest is SHACL-validated on load:
if this walker emits something malformed, Quipu rejects it.

    ./scripts/ingest-repos.py ../quipu ../hank > /tmp/code.ttl
    quipu knot /tmp/code.ttl --shapes code-entities

Predicate choice is constrained by shapes that fire on ANY subject
(`sh:targetSubjectsOf`). `aegis:contains` is bound to `sh:class aegis:Bead`
in shapes/provenance.ttl, and `bobbin:` is the same IRI namespace as `aegis:`,
so a Document-contains-Section edge would violate it. Sections use
`bobbin:inDocument`, which nothing targets; symbols use `bobbin:definedIn`,
which is constrained only inside CodeSymbolShape (targeting class CodeSymbol).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import quote

BASE = "http://aegis.gastown.local/code/"
ONTOLOGY = "http://aegis.gastown.local/ontology/"

SKIP_DIRS = {
    ".git", "target", "node_modules", "venv", ".venv", "dist", "build",
    "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "vendor",
}

# symbolKind values are an sh:in enumeration in code-entities.ttl — anything
# outside this set fails validation, so the maps below only emit legal values.
RUST_KINDS = {
    "fn": "function", "struct": "struct", "enum": "enum",
    "trait": "interface", "type": "type_alias", "const": "constant",
    "static": "variable", "mod": "module",
}
PY_KINDS = {"def": "function", "class": "class"}

RUST_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?"
    r"(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?"
    r"\b(fn|struct|enum|trait|type|const|static|mod)\s+"
    r"([A-Za-z_][A-Za-z0-9_]*)"
)
PY_RE = re.compile(r"^\s*(?:async\s+)?\b(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
FENCE_RE = re.compile(r"^\s*(```|~~~)")

LANGS = {".rs": "rust", ".py": "python"}


def iri(*parts: str) -> str:
    """Build a safe IRI from path-ish parts."""
    return BASE + "/".join(quote(p, safe="") for p in parts)


def lit(text: str) -> str:
    """Escape a string for a Turtle double-quoted literal."""
    out = text.replace("\\", "\\\\").replace('"', '\\"')
    return out.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def read_lines(path: Path) -> list[str]:
    try:
        return path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []


def walk(root: Path):
    """Yield files under root, skipping vendored and build directories."""
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        if any(part in SKIP_DIRS for part in path.relative_to(root).parts):
            continue
        yield path


def emit_symbols(out, lines, module_iri, repo, rel, pattern, kinds):
    """Emit CodeSymbol instances found in a source file."""
    count = 0
    for lineno, line in enumerate(lines, start=1):
        match = pattern.match(line)
        if not match:
            continue
        keyword, name = match.group(1), match.group(2)
        kind = kinds.get(keyword)
        if kind is None:
            continue
        # Line number keeps the IRI unique: two symbols may share a name in one
        # file (methods across impl blocks), and a collision would assert two
        # values for the maxCount-1 name/symbolKind properties.
        sym = iri(repo, rel, f"{name}-L{lineno}")
        out.append(
            f"<{sym}> a bobbin:CodeSymbol ;\n"
            f'    rdfs:label "{lit(name)}" ;\n'
            f'    bobbin:name "{lit(name)}" ;\n'
            f'    bobbin:symbolKind "{kind}" ;\n'
            f"    bobbin:definedIn <{module_iri}> ."
        )
        count += 1
    return count


def emit_sections(out, lines, doc_iri, repo, rel):
    """Emit Section instances for markdown headings, ignoring fenced code."""
    count = 0
    in_fence = False
    for lineno, line in enumerate(lines, start=1):
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        match = HEADING_RE.match(line)
        if not match:
            continue
        depth, heading = len(match.group(1)), match.group(2).strip()
        if not heading:
            continue
        sec = iri(repo, rel, f"S{lineno}")
        out.append(
            f"<{sec}> a bobbin:Section ;\n"
            f'    rdfs:label "{lit(heading)}" ;\n'
            f'    bobbin:heading "{lit(heading)}" ;\n'
            f"    bobbin:headingDepth {depth} ;\n"
            f"    bobbin:inDocument <{doc_iri}> ."
        )
        count += 1
    return count


def ingest(root: Path, repo: str, out: list[str]) -> dict[str, int]:
    tally = {"modules": 0, "symbols": 0, "documents": 0, "sections": 0}
    for path in walk(root):
        rel = path.relative_to(root).as_posix()
        suffix = path.suffix.lower()
        lines = None

        if suffix in LANGS:
            lines = read_lines(path)
            module = iri(repo, rel)
            out.append(
                f"<{module}> a bobbin:CodeModule ;\n"
                f'    rdfs:label "{lit(path.name)}" ;\n'
                f'    bobbin:filePath "{lit(rel)}" ;\n'
                f'    bobbin:repo "{lit(repo)}" ;\n'
                f'    bobbin:language "{LANGS[suffix]}" .'
            )
            tally["modules"] += 1
            pattern, kinds = (
                (RUST_RE, RUST_KINDS) if suffix == ".rs" else (PY_RE, PY_KINDS)
            )
            tally["symbols"] += emit_symbols(
                out, lines, module, repo, rel, pattern, kinds
            )

        elif suffix == ".md":
            lines = read_lines(path)
            doc = iri(repo, rel)
            out.append(
                f"<{doc}> a bobbin:Document ;\n"
                f'    rdfs:label "{lit(path.name)}" ;\n'
                f'    bobbin:filePath "{lit(rel)}" ;\n'
                f'    bobbin:repo "{lit(repo)}" .'
            )
            tally["documents"] += 1
            tally["sections"] += emit_sections(out, lines, doc, repo, rel)

    return tally


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repos", nargs="+", help="repository roots to walk")
    parser.add_argument("-o", "--out", help="output file (default: stdout)")
    args = parser.parse_args()

    out: list[str] = [
        "@prefix bobbin: <" + ONTOLOGY + "> .",
        "@prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .",
        "",
    ]
    totals = {"modules": 0, "symbols": 0, "documents": 0, "sections": 0}

    for raw in args.repos:
        root = Path(raw).resolve()
        if not root.is_dir():
            print(f"skip: {root} is not a directory", file=sys.stderr)
            continue
        tally = ingest(root, root.name, out)
        for key, value in tally.items():
            totals[key] += value
        summary = ", ".join(f"{v} {k}" for k, v in tally.items())
        print(f"{root.name}: {summary}", file=sys.stderr)

    text = "\n".join(out) + "\n"
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)

    total = sum(totals.values())
    summary = ", ".join(f"{v} {k}" for k, v in totals.items())
    print(f"total: {total} entities ({summary})", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
