#!/usr/bin/env python3
"""Assert the invariants of aegis-ontology.shapes.ttl (aegis-z0xi / aegis-1y3q).

These shapes are reconciled to what /episode ACTUALLY emits, not to what we
wish it emitted. Every constraint here is one that live data satisfies today;
that property is easy to lose and expensive to notice, because loading a
drifted shape set is silent until someone flips validate_on_write and takes
out ingestion for every affected class.

Run:
    python3 shapes/verify_shape_invariants.py                  # static invariants
    python3 shapes/verify_shape_invariants.py --data live.nt   # + dry-run gate

Exits non-zero on violation. Static checks need no server.

WHY EACH INVARIANT EXISTS — each is a bug that actually happened:

  I1  The only sh:minCount 1 in the file are rdfs:label. Required scalar
      identity predicates (ctId, skillName, formulaName, beadId, hash, path,
      roleName, schedule, hostname, backend, pool, email, title) were required
      by the shapes and emitted by nothing: 133 violations across 11 populated
      classes, every one at 0% coverage. If you add a sh:minCount for a
      predicate, prove the emitter emits it FIRST (see I4).

  I2  Every sh:targetClass NodeShape requires rdfs:label. That is the real
      de-facto contract and the reason these shapes assert something true
      instead of asserting nothing.

  I3  No sh:targetClass outside the aegis: namespace. Guards the reification
      trap: /episode emits aegis:stmt_<hash> a rdf:Statement nodes for
      edge-confidence qualifiers. They are machine-generated and carry no
      rdfs:label BY DESIGN. A targetClass reaching rdf:Statement — or any
      blanket label rule — fails the dry run on our own machinery.

  I4  (--data) The dry run against LIVE data reports zero violations. This is
      the real gate. Per ian: compute expectations at RUN TIME; a stale audit
      table is illustrative, never the expected value. If this disagrees with
      a hand-written count, this is right.

  I5  (--guide) The extraction guide's vocabulary and this file agree. FATAL
      in the guide->shapes direction: a kind the guide tells extractors to
      emit with no sh:targetClass is unshaped emission — four kinds drifted
      that way before anyone noticed, by hand. The shapes->guide direction
      only REPORTS: the guide is method, not a mirror, and deliberately omits
      kinds it has nothing to say about (its own ruling).

  I6  (--taxonomy) Same contract against the skill-side taxonomy reference —
      the third independently-drifting copy of the list.

  I7  (--live) Every kind IN LIVE USE is declared by SOME shape file here.
      I5/I6 compare documents to each other; this compares them to the GRAPH,
      and is the only check that can find an undeclared kind without a human
      noticing by hand (36 classes / 75 entities were live-but-undeclared when
      this was first measured). REPORTS rather than fails — open-world minting
      is legitimate — but the report is printed loud and last, so it cannot
      be scrolled past unseen.

The three lists compared here drift INDEPENDENTLY — that is the disease. A
generator can come later; this check stays either way, because the first
hand-edit of a generated table re-opens the same hole, silently.
"""
import json
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

SHAPES = Path(__file__).resolve().parent / "aegis-ontology.shapes.ttl"
# Untargeted PropertyShapes are inert by construction; they validate nothing.
# Deliberate (aegis-7hgo), so they are exempt from I2/I3 — see the file's
# "INERT BY CONSTRUCTION" note.
LABEL_EXEMPT = {"aegis:LabelRequiredShape"}


def parse(path):
    """Yield (shape_name, targetclass|None, [(path, [constraints])]) per shape."""
    text = path.read_text()
    shapes, cur = [], None
    for line in text.splitlines():
        m = re.match(r"^(aegis:\w+Shape)\s+a\s+sh:(NodeShape|PropertyShape)", line)
        if m:
            cur = {"name": m.group(1), "kind": m.group(2), "target": None, "props": []}
            shapes.append(cur)
            continue
        if cur is None:
            continue
        t = re.search(r"sh:targetClass\s+(\S+?)\s*[;.]", line)
        if t:
            cur["target"] = t.group(1)
        p = re.search(r"sh:path\s+(\S+?)\s*[;.]", line)
        if p:
            cur["props"].append({"path": p.group(1), "minCount": False})
        if "sh:minCount 1" in line and cur["props"]:
            cur["props"][-1]["minCount"] = True
    return shapes


AEGIS_NS = "http://aegis.gastown.local/ontology/"


def targetclasses(ttl_text):
    """Every LOCAL NAME in the aegis namespace that appears as a
    sh:targetClass — resolved through the file's OWN @prefix bindings, not by
    matching the literal string 'aegis:'. code-entities.ttl binds `bobbin:` to
    the same namespace; a prefix-blind parse silently drops its five kinds and
    then reports the whole code plane as undeclared (caught on this check's
    first live run)."""
    prefixes = dict(re.findall(r"@prefix\s+([A-Za-z][\w-]*):\s+<([^>]+)>", ttl_text))
    ns_prefixes = {p for p, ns in prefixes.items() if ns == AEGIS_NS}
    out = set()
    for pfx, local, full in re.findall(
        r"sh:targetClass\s+(?:([A-Za-z][\w-]*):([A-Za-z_]+)|<([^>]+)>)", ttl_text
    ):
        if full:
            if full.startswith(AEGIS_NS):
                out.add(full[len(AEGIS_NS):])
        elif pfx in ns_prefixes:
            out.add(local)
    return out


def doc_kinds(path, section_res, leading_only=False):
    """CamelCase `backtick` tokens inside the vocabulary section(s) of a doc.

    section_res: list of regexes; a section starts at a heading matching one
    and ends at the next '## ' heading. Only backticked TitleCase identifiers
    count — prose mentions outside backticks never do.

    leading_only: take just the FIRST backticked token of each `- ` list item.
    The guide's label lines carry example VALUES in later backticks
    ("- `SoftwareVersion` — ... (`Dolt`)"); grabbing them all turned an
    example into a phantom kind on this check's first live run.
    """
    kinds, active = set(), False
    for line in Path(path).read_text().splitlines():
        if line.startswith("## "):
            active = any(re.search(rx, line) for rx in section_res)
            continue
        if not active:
            continue
        if leading_only:
            m = re.match(r"-\s+`([A-Z][A-Za-z]+)`", line.strip())
            if m:
                kinds.add(m.group(1))
        else:
            kinds.update(re.findall(r"`([A-Z][A-Za-z]+)`", line))
    return kinds


def doc_vs_shapes(inv, doc_name, doc_set, shape_set, failures, reports):
    """Both directions. doc->shapes is FATAL (unshaped emission — the drift
    that shipped four times); shapes->doc is a REPORT (the docs are method,
    not mirrors, by their own ruling)."""
    missing_shapes = sorted(doc_set - shape_set)
    if missing_shapes:
        failures.append(
            f"{inv} {doc_name} tells extractors to emit kind(s) with NO sh:targetClass: "
            f"{', '.join(missing_shapes)}. Either shape them here or remove them from "
            f"the document — an unshaped kind in the vocabulary is exactly the drift "
            f"this check exists to catch."
        )
    undocumented = sorted(shape_set - doc_set)
    if undocumented:
        reports.append(
            f"{inv} note: {len(undocumented)} shaped kind(s) absent from {doc_name} "
            f"(legitimate — it is method, not a mirror): {', '.join(undocumented)}"
        )
    return not missing_shapes


def live_kinds(url):
    """(kind, entity_count) for every aegis-namespace class in live use."""
    query = (
        "SELECT ?c (COUNT(?s) AS ?n) WHERE { ?s "
        "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?c } GROUP BY ?c"
    )
    req = urllib.request.Request(
        url.rstrip("/") + "/query",
        data=json.dumps({"query": query}).encode(),
        headers={"Content-Type": "application/json"},
    )
    rows = json.load(urllib.request.urlopen(req, timeout=120)).get("rows", [])
    out = {}
    for r in rows:
        c = r.get("c", "")
        # aegis-namespace, but never the machine-generated code plane: those
        # kinds are declared in code-entities.ttl and their IRIs are opaque.
        m = re.match(r"http://aegis\.gastown\.local/ontology/([A-Za-z_]+)$", c)
        if m:
            out[m.group(1)] = int(r.get("n", 0))
    return out


def flag_value(flag):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else None


def main():
    if "--selftest" in sys.argv:
        sys.exit(selftest())
    if not SHAPES.exists():
        sys.exit(f"FAIL: {SHAPES} not found")
    shapes = parse(SHAPES)
    targeted = [s for s in shapes if s["target"]]
    failures = []

    # I1 — only rdfs:label may be required.
    for s in shapes:
        for p in s["props"]:
            if p["minCount"] and p["path"] != "rdfs:label":
                failures.append(
                    f"I1 {s['name']} requires {p['path']} (sh:minCount 1). Nothing but "
                    f"rdfs:label may be required unless /episode provably emits it — "
                    f"verify coverage against the live graph before adding this back."
                )

    # I2 — every targetClass shape requires rdfs:label.
    for s in targeted:
        if s["name"] in LABEL_EXEMPT:
            continue
        if not any(p["path"] == "rdfs:label" and p["minCount"] for p in s["props"]):
            failures.append(
                f"I2 {s['name']} (targetClass {s['target']}) has no "
                f"'sh:path rdfs:label ; sh:minCount 1' block — it asserts nothing."
            )

    # I3 — no targetClass outside aegis:, or the reification nodes get caught.
    for s in targeted:
        if not s["target"].startswith("aegis:"):
            failures.append(
                f"I3 {s['name']} targets {s['target']} — non-aegis targetClass. "
                f"rdf:Statement reification nodes (aegis:stmt_<hash>) carry no "
                f"rdfs:label by design; targeting them fails the dry run on our "
                f"own machinery."
            )

    print(f"parsed {len(shapes)} shapes ({len(targeted)} with sh:targetClass)")
    if not failures:
        print("I1 only rdfs:label is required .............. ok")
        print("I2 every targetClass requires rdfs:label .... ok")
        print("I3 all targetClass are aegis:-scoped ........ ok")

    # I4 — the live gate.
    if "--data" in sys.argv:
        data = sys.argv[sys.argv.index("--data") + 1]
        r = subprocess.run(
            ["quipu", "validate", "--shapes", str(SHAPES), "--data", data],
            capture_output=True, text=True,
        )
        out = (r.stdout + r.stderr).strip()
        first = out.splitlines()[0] if out else "(no output)"
        if "valid" in first and "invalid" not in first:
            print(f"I4 dry run vs live data ..................... ok ({first})")
        else:
            failures.append(
                f"I4 dry run is NOT clean: {first}\n"
                f"   Do NOT relax shapes to make this pass. A non-zero count means the "
                f"data or the emitter changed — report per-shape counts on aegis-1y3q "
                f"and stop (ian owns the contract)."
            )

    reports = []
    authority = targetclasses(SHAPES.read_text())

    # I5 — the extraction guide agrees with the shapes (guide->shapes fatal).
    guide = flag_value("--guide")
    if guide:
        gset = doc_kinds(guide, [r"Entity Labels"], leading_only=True)
        ok = doc_vs_shapes("I5", "the extraction guide", gset, authority, failures, reports)
        if ok:
            print(f"I5 guide vocabulary is shaped ............... ok ({len(gset)} kinds)")
    else:
        print("I5 guide<->shapes ........................... SKIPPED (no --guide)")

    # I6 — the skill taxonomy agrees with the shapes (taxonomy->shapes fatal).
    taxonomy = flag_value("--taxonomy")
    if taxonomy:
        tset = doc_kinds(taxonomy, [r"Entity `type` values"])
        ok = doc_vs_shapes("I6", "the skill taxonomy", tset, authority, failures, reports)
        if ok:
            print(f"I6 taxonomy vocabulary is shaped ............ ok ({len(tset)} kinds)")
    else:
        print("I6 taxonomy<->shapes ........................ SKIPPED (no --taxonomy)")

    # I7 — every kind in LIVE USE is declared by SOME shape file here.
    live_url = flag_value("--live")
    if live_url:
        declared = set()
        for ttl in SHAPES.parent.glob("*.ttl"):
            declared |= targetclasses(ttl.read_text())
        in_use = live_kinds(live_url)
        undeclared = {k: n for k, n in in_use.items() if k not in declared}
        if undeclared:
            total = sum(undeclared.values())
            reports.append(
                f"I7 LIVE-BUT-UNDECLARED: {len(undeclared)} kind(s) in use with no shape "
                f"anywhere in shapes/ ({total} entities):\n      "
                + "\n      ".join(
                    f"{k}: {n}" for k, n in sorted(undeclared.items(), key=lambda kv: -kv[1])
                )
                + "\n      Open-world minting is legitimate, so this does not fail — but "
                "every kind here is invisible to validation until someone shapes it or "
                "retires it."
            )
        else:
            print(f"I7 live kinds all declared .................. ok ({len(in_use)} in use)")
    else:
        print("I7 live<->declared .......................... SKIPPED (no --live)")

    if reports:
        print("\n" + "=" * 66)
        print("REPORTS (non-fatal, do not scroll past):")
        for r in reports:
            print(f"  - {r}")
        print("=" * 66)

    if failures:
        print(f"\nFAILED — {len(failures)} invariant violation(s):\n", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        sys.exit(1)
    print("\nall invariants hold")


def selftest():
    """Both-outcomes proof for I5/I6/I7 — a guard that has never been seen
    failing is indistinguishable from no guard. Synthesizes fixtures; touches
    no live system."""
    import tempfile

    fails = []
    authority = {"LXCContainer", "CrewMember"}

    def run_doc(inv, doc_text, section, leading_only=False):
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
            f.write(doc_text)
            name = f.name
        failures, reports = [], []
        ok = doc_vs_shapes(
            inv, "fixture", doc_kinds(name, [section], leading_only), authority, failures, reports
        )
        Path(name).unlink()
        return ok, failures, reports

    # I5/I6 negative: a documented kind with no shape MUST fail.
    ok, failures, _ = run_doc(
        "I5", "## Entity Labels\n- `LXCContainer` — x\n- `Unshaped` — y\n## Next\n", r"Entity Labels"
    )
    if ok or not any("Unshaped" in f for f in failures):
        fails.append("I5-negative: an unshaped documented kind did NOT fail")

    # I5/I6 positive: agreement passes; shapes-only kinds only REPORT.
    ok, failures, reports = run_doc(
        "I5", "## Entity Labels\n- `LXCContainer` — x\n## Next\n", r"Entity Labels"
    )
    if not ok or failures:
        fails.append("I5-positive: agreement was scored as failure")
    if not any("CrewMember" in r for r in reports):
        fails.append("I5-report: a shaped-but-undocumented kind was not reported")

    # Section scoping: kinds OUTSIDE the vocabulary section must not count.
    ok, failures, _ = run_doc(
        "I5", "## Prose\n`Unshaped` mentioned in passing\n## Entity Labels\n- `LXCContainer`\n", r"Entity Labels"
    )
    if not ok:
        fails.append("I5-scope: a backticked kind outside the section was counted")

    # Example values must not become phantom kinds (leading_only mode).
    ok, failures, _ = run_doc(
        "I5", "## Entity Labels\n- `LXCContainer` — x (`Dolt`)\n## Next\n",
        r"Entity Labels", leading_only=True,
    )
    if not ok:
        fails.append("I5-example: a backticked example value was counted as a kind")

    # targetclasses() must resolve ANY prefix bound to the aegis namespace,
    # and full-IRI form — not just the literal 'aegis:'.
    ttl = (
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n"
        f"@prefix bobbin: <{AEGIS_NS}> .\n"
        "@prefix aegis: <" + AEGIS_NS + "> .\n"
        "@prefix other: <http://example.org/> .\n"
        "x sh:targetClass bobbin:CodeSymbol .\n"
        "y sh:targetClass aegis:Bead .\n"
        f"z sh:targetClass <{AEGIS_NS}Metric> .\n"
        "w sh:targetClass other:Foreign .\n"
    )
    got = targetclasses(ttl)
    if got != {"CodeSymbol", "Bead", "Metric"}:
        fails.append(f"targetclasses prefix resolution wrong: {sorted(got)}")

    # I7 negative: an in-use kind absent from every declared set must surface.
    declared = {"LXCContainer"}
    in_use = {"LXCContainer": 30, "GhostKind": 3}
    undeclared = {k: n for k, n in in_use.items() if k not in declared}
    if undeclared != {"GhostKind": 3}:
        fails.append("I7-negative: the live-but-undeclared kind was not isolated")

    if fails:
        print("SELFTEST FAILED:", file=sys.stderr)
        for f in fails:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("SELFTEST PASSED: I5/I6 fail on unshaped documented kinds, pass on "
          "agreement, report shapes-only kinds, scope to the vocabulary "
          "section; I7 isolates live-but-undeclared kinds.")
    return 0


if __name__ == "__main__":
    main()
