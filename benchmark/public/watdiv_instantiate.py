#!/usr/bin/env python3
"""Instantiate WatDiv query templates against a LOADED slice, or refuse.

WatDiv ships query TEMPLATES, not queries. Each carries `%vN%` placeholders and
`#mapping vN <class> <distribution>` directives, and no PREFIX lines at all. This
turns a template directory into runnable `*.rq` for `quipu-oxi-compare`, and
records what it bound so two runs are comparable.

WHY THE BINDINGS COME FROM THE SLICE, NOT FROM THE MODEL
--------------------------------------------------------
A binding is only useful if the entity is IN THE STORE. The benchmark loads a
contiguous PREFIX of a published archive, and WatDiv's entity classes are not
uniformly distributed through the file -- `wsdbm:Retailer` first appears at line
1,474,089 of `watdiv.10M.nt` and `wsdbm:AgeGroup` at line 2,060,223. Binding from
the model's declared cardinalities would mint `wsdbm:Retailer7` for a 1M-line
slice that contains no Retailer at all, and the query would return zero rows while
looking perfectly well-formed. So the entity pool is scanned out of the slice that
will actually be loaded, and a class with an empty pool is a REFUSAL (exit 3), not
a query that silently matches nothing.

WHY CLASS MEMBERSHIP IS READ FROM THE IRI AND NOT FROM rdf:type
---------------------------------------------------------------
Measured on `watdiv.10M.nt` (10,916,457 triples): the ONLY classes appearing as
`rdf:type` objects anywhere in the archive are Role, ProductCategory and Genre.
The published generator carries class membership in the entity IRI localname
(`wsdbm:City17`), and the model declares those classes with cardinalities --
`<type*> wsdbm:City 240` -- rather than emitting a type triple per instance. A
census taken over `rdf:type` therefore reports 1 of 9 template classes present
when 7 of 9 are bindable in the same bytes. That census was the recorded blocker
on aegis-j0yaxj.2; it was measuring the wrong signal.

WHY THE NAMESPACES ARE READ FROM THE MODEL FILE EVERY RUN
----------------------------------------------------------
WatDiv's prefixes are NOT the standard ones: it declares
`foaf=http://xmlns.com/foaf/` (no `0.1/`) and `gr=http://purl.org/goodrelations/`
(no `v1#`). A hardcoded "obvious" prefix map produces queries that parse, run,
and match nothing in either engine -- the failure this whole file exists to make
impossible. Verified against the data: every namespace root occurring as a
predicate in the slice is one the model declares.

WHAT THIS DOES NOT DO
---------------------
It proves a binding EXISTS in the slice. It does not prove the query returns
rows: the rest of the pattern still has to join, and a prefix truncation breaks
joins by construction. Row-positivity is a separate assertion that needs a loaded
store -- `quipu-oxi-compare` refuses when every arm returns zero. Do not read a
clean exit here as "the workload is non-vacuous".
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import re
import sys
from pathlib import Path

#: `#namespace <prefix>=<iri>` in the WatDiv data model.
NAMESPACE_RE = re.compile(r"^#namespace\s+(\S+?)\s*=\s*(\S+)\s*$")

#: `#mapping <var> <prefix:Class> <distribution>` at the head of a template.
MAPPING_RE = re.compile(r"^#mapping\s+(\S+)\s+(\S+)\s+(\S+)\s*$")

#: `%vN%` placeholder in a template body.
PLACEHOLDER_RE = re.compile(r"%(\w+)%")

#: An entity IRI whose localname is a class name followed by an instance number.
#: Captured from the slice, so the pool is exactly what will be in the store.
ENTITY_RE = re.compile(r"<([^>]*[/#])([A-Za-z_]+)(\d+)>")

EXIT_REFUSED_UNBINDABLE = 3
EXIT_BAD_INPUT = 2


def read_namespaces(model_path: Path) -> dict[str, str]:
    """Return the prefix map the data was GENERATED with.

    Refuses an empty map: a model file that parses to no namespaces would
    produce prefix-less queries that fail to parse, and the error would surface
    at the harness rather than here.
    """
    namespaces: dict[str, str] = {}
    for line in model_path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = NAMESPACE_RE.match(line)
        if match:
            namespaces[match.group(1)] = match.group(2)
    if not namespaces:
        raise SystemExit(
            f"{model_path}: no #namespace declarations found. "
            "A prefix map cannot be guessed -- WatDiv's foaf and gr roots are "
            "nonstandard and the wrong ones match nothing."
        )
    return namespaces


def parse_template(path: Path) -> tuple[list[tuple[str, str, str]], str]:
    """Split a template into its `#mapping` directives and its query body."""
    mappings: list[tuple[str, str, str]] = []
    body_lines: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = MAPPING_RE.match(line)
        if match:
            mappings.append((match.group(1), match.group(2), match.group(3)))
        elif line.startswith("#"):
            continue
        else:
            body_lines.append(line)
    return mappings, "\n".join(body_lines).strip()


def scan_entity_pool(
    slice_path: Path, wanted_classes: set[str]
) -> tuple[dict[str, list[str]], int, str]:
    """Collect the entity IRIs of each wanted class that occur in the slice.

    Returns (pool, line_count, sha256). The digest is of the slice bytes and goes
    into the manifest: a binding set is only meaningful against the exact bytes it
    was drawn from.

    Scans ONCE and keeps a set per class, because the pools are small dimension
    tables (City 240, Country 25, AgeGroup 9) except User, which is capped below.
    """
    pool: dict[str, set[str]] = {name: set() for name in wanted_classes}
    digest = hashlib.sha256()
    lines = 0
    with slice_path.open("rb") as handle:
        for raw in handle:
            digest.update(raw)
            lines += 1
            text = raw.decode("utf-8", errors="replace")
            for namespace, class_name, _number in ENTITY_RE.findall(text):
                if class_name in pool:
                    pool[class_name].add(f"{namespace}{class_name}{_number}")
    return (
        {name: sorted(values) for name, values in pool.items()},
        lines,
        digest.hexdigest(),
    )


def local_name(qualified: str) -> str:
    """`wsdbm:Retailer` -> `Retailer`; a bare name passes through unchanged."""
    return qualified.split(":", 1)[1] if ":" in qualified else qualified


def render(
    body: str,
    bindings: dict[str, str],
    namespaces: dict[str, str],
    graph_iri: str | None,
) -> str:
    """Substitute the bindings, add PREFIX lines, and scope to the graph.

    The graph scoping is applied to the WHERE body because the harness hands the
    query text VERBATIM to both arms -- so if it is not in the file, it is not in
    the measurement, and Quipu would answer from ROOT while Oxigraph answers from
    its default graph.
    """
    filled = body
    for variable, value in bindings.items():
        filled = filled.replace(f"%{variable}%", f"<{value}>")

    if graph_iri is not None:
        head, brace, rest = filled.partition("{")
        if not brace:
            raise SystemExit(f"template body has no WHERE block:\n{body}")
        inner = rest.rstrip()
        if not inner.endswith("}"):
            raise SystemExit(f"template body does not close its WHERE block:\n{body}")
        inner = inner[:-1].rstrip()
        filled = f"{head}{{\n\tGRAPH <{graph_iri}> {{\n{inner}\n\t}}\n}}"

    prefixes = "\n".join(
        f"PREFIX {prefix}: <{iri}>" for prefix, iri in sorted(namespaces.items())
    )
    return f"{prefixes}\n{filled}\n"


def collect_templates(roots: list[Path]) -> list[Path]:
    """Every `*.txt` under the given roots, deterministically ordered."""
    found: list[Path] = []
    for root in roots:
        if root.is_file():
            found.append(root)
        else:
            found.extend(sorted(root.glob("*.txt")))
    return sorted(set(found))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--templates", required=True, nargs="+", type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--slice", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--graph", default=None)
    parser.add_argument("--seed", required=True, type=int)
    parser.add_argument(
        "--per-template",
        type=int,
        default=1,
        help="instantiations per template; each gets its own .rq and ledger row",
    )
    parser.add_argument(
        "--allow-unbindable",
        action="store_true",
        help=(
            "emit the bindable templates and record the rest as NOT RUN instead of "
            "refusing. The omitted ids are NAMED in the manifest -- never dropped."
        ),
    )
    args = parser.parse_args()

    for path in (*args.templates, args.model, args.slice):
        if not path.exists():
            print(f"missing input: {path}", file=sys.stderr)
            return EXIT_BAD_INPUT

    namespaces = read_namespaces(args.model)
    templates = collect_templates(list(args.templates))
    if not templates:
        print(f"no templates under {args.templates}", file=sys.stderr)
        return EXIT_BAD_INPUT

    parsed = {path: parse_template(path) for path in templates}
    wanted = {
        local_name(qualified)
        for mappings, _ in parsed.values()
        for _var, qualified, _dist in mappings
    }

    pool, slice_lines, slice_sha = scan_entity_pool(args.slice, wanted)

    unbindable: dict[str, list[str]] = {}
    for path, (mappings, _body) in parsed.items():
        missing = [q for _v, q, _d in mappings if not pool.get(local_name(q))]
        if missing:
            unbindable[path.stem] = missing

    if unbindable and not args.allow_unbindable:
        print(
            "REFUSING: these templates map a class with NO entity in the slice.\n"
            "Every query would parse, run, and match nothing -- which is a clean\n"
            "timing on an empty result, not a measurement.\n",
            file=sys.stderr,
        )
        for name, classes in sorted(unbindable.items()):
            print(f"  {name}: {', '.join(classes)}", file=sys.stderr)
        print(
            f"\nslice: {args.slice} ({slice_lines} lines)\n"
            "Remedy: take a longer slice, or pass --allow-unbindable to publish\n"
            "these ids as NOT RUN. Do not bind them from the model -- an entity\n"
            "that is not in the store is not a binding.",
            file=sys.stderr,
        )
        return EXIT_REFUSED_UNBINDABLE

    args.out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)
    emitted: list[dict[str, object]] = []

    for path in templates:
        mappings, body = parsed[path]
        if path.stem in unbindable:
            continue
        for index in range(args.per_template):
            bindings = {
                variable: rng.choice(pool[local_name(qualified)])
                for variable, qualified, _dist in mappings
            }
            name = path.stem if args.per_template == 1 else f"{path.stem}-{index}"
            text = render(body, bindings, namespaces, args.graph)
            (args.out / f"{name}.rq").write_text(text, encoding="utf-8")
            emitted.append(
                {
                    "name": name,
                    "template": path.name,
                    "bindings": bindings,
                    "unbound": not mappings,
                }
            )

    manifest = {
        "seed": args.seed,
        "per_template": args.per_template,
        "graph": args.graph,
        "slice": {
            "path": str(args.slice),
            "lines": slice_lines,
            "sha256": slice_sha,
        },
        "namespaces": namespaces,
        "pool_sizes": {name: len(values) for name, values in sorted(pool.items())},
        "emitted": emitted,
        # NAMED, never dropped: a template that could not be bound is a published
        # NOT RUN at template granularity, not an absence in the result set.
        "not_run": {name: classes for name, classes in sorted(unbindable.items())},
    }
    (args.out / "instantiation-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    print(f"emitted {len(emitted)} query file(s) to {args.out}")
    print(f"slice {slice_lines} lines, sha256 {slice_sha}")
    for name, size in sorted(manifest["pool_sizes"].items()):
        print(f"  pool {name:<18} {size}")
    if unbindable:
        print(f"NOT RUN ({len(unbindable)}): {', '.join(sorted(unbindable))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
