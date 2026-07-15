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
"""
import re
import subprocess
import sys
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


def main():
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

    if failures:
        print(f"\nFAILED — {len(failures)} invariant violation(s):\n", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        sys.exit(1)
    print("\nall invariants hold")


if __name__ == "__main__":
    main()
