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

  I1  The only sh:minCount 1 in the file are rdfs:label, unless the requirement
      is backed either by measured 100% emitter coverage for a populated class,
      or by schema-first conformance fixtures for a brand-new class with zero
      instances. Required scalar
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

  I8  (--live) No ABSTRACT parent is asserted directly on an instance. An
      abstract parent is DERIVED, not listed: a class used as the object of
      rdfs:subClassOf that has no sh:targetClass of its own (today:
      FailureKnowledge, OperationalKnowledge, TextRule — none asserted).
      I7 cannot see this — the parent IS declared, just not shaped —
      and neither can any coverage count, because a constant-IRI type query
      resolves subclasses and returns the node whether it was typed with the
      parent or the child. So `?s a aegis:Service` reports a node as covered
      while nothing validates it; the gap is invisible from every angle that
      counts (aegis-19gip, where it was reported as 346 ungoverned Service
      nodes, then re-measured to 61 asserted, of which 26 carried no other
      type at all — those 26 are governed by nothing whatsoever).

      Judgement, RESOLVED and now FATAL. This paragraph used to
      record the opposite ruling — "Service and Host stay abstract and UNSHAPED,
      the fix is retyping" — and it was already contradicted by the file it
      guards: ec1f082 shaped Host, for the reason retyping was rejected. Both
      remedies are legitimate; which one applies is a per-class call, so a
      blanket ruling in the docstring was the wrong instrument and went stale
      the same day it was written.

      The call actually made, per class, on measurement:
        Host, Service, Tool — SHAPED (rdfs:label floor). Each has children that
          already resolve to it, so the constant-IRI query expands to the whole
          population; retyping the asserted ones away would break that for the
          bare nodes, which are the population no child class exists for (26 of
          65 Service, 11 of 18 Tool). Label coverage was 100% before landing, so
          none of the three shapes refuses anything that exists.
        Retyping stays the right remedy where the assertions are genuinely
          MISFILED rather than merely broad. Nothing is in that state today.

      Direct assertions are therefore ZERO, which is the condition this check
      set for itself, so it now FAILS instead of reporting. A report is what let
      the backlog re-grow; a failure is what stops it. If this fires, the fix is
      the per-class call above — never widening the exemption.

The three lists compared here drift INDEPENDENTLY — that is the disease. A
generator can come later; this check stays either way, because the first
hand-edit of a generated table re-opens the same hole, silently.
"""
import inspect
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

AEGIS_NS = "http://aegis.gastown.local/ontology/"

# I1 escape hatch, and the ONLY one (aegis-vt03v).
#
# I1's own rule is not "never require anything" -- it is "prove the emitter
# emits it FIRST". Until now there was nowhere to RECORD that proof, so a
# requirement that had been proven looked identical to one somebody added on a
# hunch, and the gate stayed red with no way to distinguish them.
#
# An entry here is normally a claim that live coverage was MEASURED AT 100%.
# For a brand-new, zero-instance class that must exist before its emitter, a
# schema-first contract is allowed only when tests prove the complete valid
# record and every required-field negative arm. Carry which proof applies.
#
#   POST /query on the quipu server:
#     SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE {
#       ?s a/rdfs:subClassOf* aegis:<Class> ; aegis:<predicate> ?v }
#   compared against the same query without the predicate clause.
#
# Do NOT add an entry to make a red run green. The 133-violation incident in I1
# above is what that produces: shapes requiring predicates nothing emitted, and
# ingestion for 11 classes one validate_on_write flag away from dying.
REQUIRED_PREDICATE_PROVEN = {
    # (shape, predicate): "<measured coverage> <date> <who>"
    ("aegis:TextRuleShape", "aegis:regex"):
        "7/7 TextRule instances (via rdfs:subClassOf*) 2026-08-24 grant",
    ("aegis:TextRuleShape", "aegis:enforcementTier"):
        "7/7 TextRule instances (via rdfs:subClassOf*) 2026-08-24 grant",
    ("aegis:CommandDiskImpactObservationShape", "aegis:commandSignature"):
        "schema-first zero-instance contract + positive/negative fixtures 2026-09-02 ian",
    ("aegis:CommandDiskImpactObservationShape", "aegis:filesystemIdentity"):
        "schema-first zero-instance contract + positive/negative fixtures 2026-09-02 ian",
    ("aegis:CommandDiskImpactObservationShape", "aegis:diskDeltaBytes"):
        "schema-first zero-instance contract + positive/negative fixtures 2026-09-02 ian",
    ("aegis:CommandDiskImpactObservationShape", "aegis:observedAt"):
        "schema-first zero-instance contract + positive/negative fixtures 2026-09-02 ian",
}


def parse(path):
    """Yield (shape_name, targetclass|None, [(path, [constraints])]) per shape.

    PREFIX-AWARE, resolved through the file's OWN @prefix bindings (aegis-vt03v).
    This used to match a literal `^aegis:` and was therefore blind to any file
    that binds the aegis namespace under a different prefix — exactly the trap
    targetclasses() below documents having already been caught by. MEASURED
    2026-08-24: code-entities.ttl binds `bobbin:` to the aegis namespace and
    declares 7 NodeShapes; the literal-prefix parse saw ZERO of them. So the
    code plane could not be examined even when this function was pointed
    straight at its file.
    """
    text = path.read_text()
    prefixes = dict(re.findall(r"@prefix\s+([A-Za-z][\w-]*):\s+<([^>]+)>", text))
    ns_prefixes = {p for p, ns in prefixes.items() if ns == AEGIS_NS}
    shapes, cur = [], None
    for line in text.splitlines():
        m = re.match(r"^([A-Za-z][\w-]*):(\w+Shape)\s+a\s+sh:(NodeShape|PropertyShape)", line)
        if m and m.group(1) not in ns_prefixes:
            m = None            # a shape in some OTHER namespace is not ours to judge
        if m:
            cur = {"name": f"{m.group(1)}:{m.group(2)}", "kind": m.group(3),
                   "target": None, "props": []}
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


def parse_text_for_selftest(text):
    """parse() over a literal string — selftest only, so the namespace-scoping
    half of the prefix rule can be asserted without a fixture file on disk."""
    import tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".ttl", delete=False) as fh:
        fh.write(text)
        tmp = Path(fh.name)
    try:
        return parse(tmp)
    finally:
        tmp.unlink(missing_ok=True)


def other_shape_files():
    """Every shapes/*.ttl except the one I1/I2/I3 actually read."""
    return sorted(p for p in SHAPES.parent.glob("*.ttl") if p != SHAPES)


def label_floor_report(paths):
    """I9 (aegis-vt03v). What I1/I2 WOULD say about the shape files they do not
    read — reported, never fatal.

    NOT a widened I1/I2, deliberately. I1 ("only rdfs:label may be required") is
    an aegis-ontology.shapes.ttl POLICY and was never meant to be global:
    governance.ttl's Verdict genuinely requires a signature and an evidenceHash
    because those are machine-emitted records, not human-facing entities. Made
    fatal across every file this would fail on ~50 deliberate constraints, and a
    check that is wrong 50 times gets switched off, taking the 2 real findings
    with it.

    So this reports two things per file and rules on neither:
      * targetClass shapes with NO rdfs:label floor  (what I2 would flag)
      * required predicates other than rdfs:label    (what I1 would flag)
    """
    out = []
    for path in paths:
        shapes = parse(path)
        targeted = [s for s in shapes if s["target"]]
        if not shapes:
            continue
        no_label = [
            s["name"] for s in targeted
            if s["name"] not in LABEL_EXEMPT
            and not any(p["path"] == "rdfs:label" and p["minCount"] for p in s["props"])
        ]
        required = [
            f"{s['name']}:{p['path']}" for s in shapes for p in s["props"]
            if p["minCount"] and p["path"] != "rdfs:label"
        ]
        if no_label:
            out.append(
                f"I9 {path.name}: {len(no_label)} targetClass shape(s) with no "
                f"rdfs:label floor — {', '.join(sorted(no_label))}. I2 does not "
                f"read this file, so a reader of 'I2 ... ok' is not being told "
                f"about these."
            )
        if required:
            out.append(
                f"I9 {path.name}: {len(required)} required predicate(s) other than "
                f"rdfs:label — {', '.join(sorted(required))}. Many are deliberate "
                f"(machine-emitted records); I1's policy is scoped to "
                f"{SHAPES.name} and is NOT asserted over this file."
            )
    return out


def subclass_parents(ttl_text):
    """Every aegis-namespace LOCAL NAME used as the OBJECT of rdfs:subClassOf.
    Same prefix-resolution rule as targetclasses() — and the same trap, since
    governance.ttl declares three of these under its own bindings."""
    prefixes = dict(re.findall(r"@prefix\s+([A-Za-z][\w-]*):\s+<([^>]+)>", ttl_text))
    ns_prefixes = {p for p, ns in prefixes.items() if ns == AEGIS_NS}
    out = set()
    for pfx, local, full in re.findall(
        r"rdfs:subClassOf\s+(?:([A-Za-z][\w-]*):([A-Za-z_]+)|<([^>]+)>)", ttl_text
    ):
        if full:
            if full.startswith(AEGIS_NS):
                out.add(full[len(AEGIS_NS):])
        elif pfx in ns_prefixes:
            out.add(local)
    return out


def abstract_parents(parents, declared):
    """Parents with no sh:targetClass anywhere — DERIVED, never listed.

    A hand-maintained list of abstract classes is the stale audit table I4
    forbids, one abstraction up. Deriving it means shaping a parent is the
    single act that retires it from this check, and adding a parent enrols
    it automatically."""
    return parents - declared


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
    proven = []

    # I1 — only rdfs:label may be required, unless the requirement is justified.
    for s in shapes:
        for p in s["props"]:
            if p["minCount"] and p["path"] != "rdfs:label":
                if (s["name"], p["path"]) in REQUIRED_PREDICATE_PROVEN:
                    proven.append(
                        f"I1 {s['name']} requires {p['path']} — allowed, requirement "
                        f"justified: {REQUIRED_PREDICATE_PROVEN[(s['name'], p['path'])]}"
                    )
                    continue
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

    scope = SHAPES.name
    print(f"parsed {len(shapes)} shapes ({len(targeted)} with sh:targetClass) "
          f"in {scope}")
    for line in proven:
        print(f"  (proven) {line}")
    if not failures:
        # NAME THE FILE. These three read ONE file while I7/I8 glob shapes/*.ttl,
        # and saying "every targetClass" of a single-file check is a POSITIVE
        # CLAIM OF COVERAGE THAT STOPS ANYONE LOOKING (aegis-vt03v). It is not
        # academic: bobbin:SectionShape lives in code-entities.ttl, requires
        # bobbin:heading and bobbin:headingDepth and requires no label -- I1 and
        # I2 would each have caught one half -- and 1751 Section nodes sat with
        # no rdfs:label while this printed "ok" for months.
        print(f"I1 only rdfs:label is required .............. ok [{scope}]")
        print(f"I2 every targetClass requires rdfs:label .... ok [{scope}]")
        print(f"I3 all targetClass are aegis:-scoped ........ ok [{scope}]")

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

        # I8 — an ABSTRACT parent must not be asserted directly on an instance.
        parents = set()
        for ttl in SHAPES.parent.glob("*.ttl"):
            parents |= subclass_parents(ttl.read_text())
        abstract = abstract_parents(parents, declared)
        # in_use is the ASSERTED count: live_kinds() asks `?s a ?c` with ?c a
        # VARIABLE, which quipu does NOT expand over subClassOf. The constant
        # form would return the children too and every abstract parent would
        # look catastrophically violated (aegis-6h45e).
        direct = {k: in_use[k] for k in sorted(abstract) if in_use.get(k)}
        if direct:
            # FATAL since ServiceShape and ToolShape landed. This reported for
            # as long as live data did not satisfy it (the I1 rule); the
            # backlog is now zero, so the promise the I8 docstring made — flip
            # this the moment the count reaches zero — comes due.
            failures.append(
                f"I8 ABSTRACT-PARENT ASSERTED DIRECTLY: {len(direct)} of {len(abstract)} "
                f"abstract parent(s) carry direct assertions ({sum(direct.values())} entities):\n      "
                + "\n      ".join(f"{k}: {n}" for k, n in sorted(direct.items(), key=lambda kv: -kv[1]))
                + "\n      These classes have children but no shape of their own, so a node typed"
                "\n      with the PARENT is governed by nothing while reading as covered — the"
                "\n      inference-expanded count includes it either way. Retype each to the"
                "\n      concrete child, or shape the parent (which retires it from this check)."
                "\n      Do NOT silence this by exempting the class: an exemption list is the"
                "\n      stale audit table I4 forbids, and the backlog it hides re-grows unseen."
            )
        elif abstract:
            print(f"I8 abstract parents never asserted .......... ok ({len(abstract)} abstract)")
    else:
        print("I7 live<->declared .......................... SKIPPED (no --live)")
        print("I8 abstract-parent assertion ................ SKIPPED (no --live)")

    # I9 — the honest edge of I1/I2's scope. Always runs: it needs no server and
    # no flag, and the whole point is that a reader of "ok" learns what was NOT
    # examined. Non-fatal by design (see label_floor_report).
    others = other_shape_files()
    i9 = label_floor_report(others)
    examined = [p.name for p in others if parse(p)]
    if i9:
        reports.extend(i9)
        print(f"I9 other shape files ........................ {len(i9)} report(s) "
              f"over {len(examined)} file(s) — see REPORTS")
    else:
        print(f"I9 other shape files ........................ ok "
              f"({len(examined)} file(s) examined)")

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

    # I8: subclass_parents() resolves any aegis-bound prefix and full IRIs, and
    # must pick up the OBJECT of subClassOf, never the subject.
    sub_ttl = (
        "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n"
        f"@prefix gov: <{AEGIS_NS}> .\n"
        "@prefix other: <http://example.org/> .\n"
        "gov:WebApplication rdfs:subClassOf gov:Service .\n"
        f"gov:ProxmoxNode rdfs:subClassOf <{AEGIS_NS}Host> .\n"
        "gov:Thing rdfs:subClassOf other:Foreign .\n"
    )
    got = subclass_parents(sub_ttl)
    if got != {"Service", "Host"}:
        fails.append(f"I8 subclass_parents wrong (subject leaked / prefix missed): {sorted(got)}")

    # I8 negative: a parent with children but NO shape is abstract and, if
    # asserted, must surface. Positive: shaping the parent retires it.
    parents, shaped = {"Service", "Host", "Directive"}, {"Directive", "WebApplication"}
    abstract = abstract_parents(parents, shaped)
    if abstract != {"Service", "Host"}:
        fails.append(f"I8-derive: abstract set wrong: {sorted(abstract)}")
    counts = {"Service": 61, "Host": 16, "Directive": 157, "WebApplication": 107}
    direct = {k: counts[k] for k in sorted(abstract) if counts.get(k)}
    if direct != {"Service": 61, "Host": 16}:
        fails.append(f"I8-negative: asserted abstract parents not isolated: {direct}")
    if {k: 0 for k in abstract if 0}:
        fails.append("I8-positive: a zero-assertion abstract parent was reported")
    if abstract_parents(parents, shaped | {"Service", "Host"}):
        fails.append("I8-positive: shaping a parent did not retire it from the check")

    # I8 SEVERITY. The backlog reached zero, so I8 fails rather
    # than reports; the failure mode this guards is someone quieting it back to
    # a report to make a red run green, which is indistinguishable from a fix in
    # the output and re-opens the hole silently. Asserted against this file's
    # OWN SOURCE and named as such: the routing lives inline in main(), so there
    # is no seam to call, and a source assertion that says so beats a
    # behavioural claim that is really only a comment.
    # Read main()'s source, NOT the whole file: this check's own marker string
    # would otherwise match itself and the guard would pass on nothing.
    i8_block = inspect.getsource(main).split("# I8 ")[-1].split("elif abstract:")[0]
    if "failures.append(" not in i8_block:
        fails.append("I8-severity: the I8 branch no longer routes to failures — "
                     "it was demoted back to a report")
    if "reports.append(" in i8_block:
        fails.append("I8-severity: the I8 branch appends to reports; a direct "
                     "assertion must be FATAL, not scrolled past")

    # ---- aegis-vt03v: scope honesty. Each case is the bug, not a paraphrase.

    # parse() must be PREFIX-AWARE. code-entities.ttl binds the aegis namespace
    # to `bobbin:`; the literal-prefix parse saw 0 of its 7 NodeShapes, so the
    # code plane was unexaminable even when pointed straight at its file.
    ce = SHAPES.parent / "code-entities.ttl"
    if ce.exists():
        ce_shapes = parse(ce)
        if len(ce_shapes) < 7:
            fails.append(
                f"vt03v-prefix: parse() saw {len(ce_shapes)} shapes in "
                f"code-entities.ttl, expected >= 7 — it is prefix-blind again"
            )
        if not any(s["name"] == "bobbin:SectionShape" for s in ce_shapes):
            fails.append("vt03v-prefix: bobbin:SectionShape not seen by parse()")
        # ...and a shape in a genuinely FOREIGN namespace must stay out of scope.
        foreign = parse_text_for_selftest(
            "@prefix sh: <http://www.w3.org/ns/shacl#> .\n"
            "@prefix other: <http://example.invalid/ns#> .\n"
            "other:ThingShape a sh:NodeShape ;\n    sh:targetClass other:Thing .\n"
        )
        if foreign:
            fails.append("vt03v-prefix: a non-aegis-namespace shape was parsed as ours")

    # The I1/I2/I3 lines must NAME THE FILE. "every targetClass ... ok" from a
    # single-file check is a positive claim of coverage that stops anyone
    # looking -- the whole defect.
    main_src = inspect.getsource(main)
    for inv in ("I1 only rdfs:label", "I2 every targetClass", "I3 all targetClass"):
        seg = main_src.split(inv)[-1].split("\n")[0]
        if "{scope}" not in seg:
            fails.append(
                f"vt03v-scope: the '{inv}' output line no longer names the file it "
                f"reads; an unscoped 'ok' reads as global coverage"
            )

    # I9 must stay NON-FATAL. Made fatal it fails on ~50 deliberate constraints,
    # and a check that is wrong 50 times gets switched off with the real ones.
    i9_seg = main_src.split("i9 = label_floor_report")[-1].split("if reports:")[0]
    if "failures.append" in i9_seg or "sys.exit" in i9_seg:
        fails.append("vt03v-i9: I9 was promoted to fatal; it reports by design")
    if "reports.extend" not in i9_seg:
        fails.append("vt03v-i9: I9 no longer feeds REPORTS — it would be invisible")

    # I9 must actually SEE the other files. A silent zero here is the same class
    # of lie as the unscoped 'ok'.
    others = other_shape_files()
    if others and not any(parse(p) for p in others):
        fails.append("vt03v-i9: I9 parsed 0 shapes across every other file")

    # Every proven-coverage entry must CARRY its measurement. An empty note
    # turns the escape hatch into the exemption list I1's history forbids.
    for key, note in REQUIRED_PREDICATE_PROVEN.items():
        if not note or not any(ch.isdigit() for ch in note):
            fails.append(
                f"vt03v-proven: {key} has no measurement recorded — an entry here "
                f"is a claim that coverage was MEASURED, so it must carry a number"
            )

    if fails:
        print("SELFTEST FAILED:", file=sys.stderr)
        for f in fails:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("SELFTEST PASSED: I5/I6 fail on unshaped documented kinds, pass on "
          "agreement, report shapes-only kinds, scope to the vocabulary "
          "section; I7 isolates live-but-undeclared kinds; I8 derives the "
          "abstract set, isolates asserted parents, and is retired by shaping; "
          "parse() resolves the aegis namespace through any prefix and rejects "
          "foreign ones, I1/I2/I3 name the file they read, I9 stays non-fatal "
          "and visible, and every proven-coverage entry carries its measurement.")
    return 0


if __name__ == "__main__":
    main()
