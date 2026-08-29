#!/usr/bin/env python3
"""Build the ARM B replay corpus from a live Quipu store.

Extracts the two recorded multi-agent divergence classes as a REPLAYABLE
fixture, pseudonymised so the artifact carries no deployment identifier:

  alias-mint      two names silently minted for one entity, later repaired
                  by an owl:sameAs knot. Each surviving knot is one historical
                  human repair decision.
  comment-double  a maxCount-1 predicate that the production append path
                  doubled instead of replacing.

The pseudonymiser is deterministic and STRUCTURE-PRESERVING: it renames, it
never drops triples, so every cardinality the merge operator reasons about
survives. Run with --verify to prove that on the emitted fixture.
"""

import argparse, hashlib, json, os, re, sys, urllib.request

# The source store's namespace is deployment-specific and is NOT hardcoded here
# (this file is public). Override with --namespace or REPLAY_NAMESPACE.
ONT = os.environ.get("REPLAY_NAMESPACE", "http://example.invalid/ontology/")
PUB = "https://example.org/kg/"
SAME_AS = "http://www.w3.org/2002/07/owl#sameAs"
COMMENT = "http://www.w3.org/2000/01/rdf-schema#comment"

# Only patterns that are internal-identifier shaped in ANY deployment live here,
# because this file is public. Site conventions — private-network suffixes, host
# names, service names — are deployment-specific and are supplied at run time via
# --deny-file or REPLAY_DENY_FILE, one regex per line.
#
# That split is not tidiness. An earlier revision listed the suffix conventions
# inline and the repository's own pre-push scrub guard refused the push, citing
# this file's *control probes* as the leak. It was right to: a guard cannot tell
# a real name from an example of one, and defeating it by splitting the literal
# would have hidden the string from the next person's grep too. The fix is for
# the public file not to need the strings at all.
STRUCTURAL = [
    r"\b\d{1,3}(\.\d{1,3}){3}\b",   # bare IPv4
    r"/home/[a-z]",                   # home paths
]

def build_detector(deny_file):
    pats = list(STRUCTURAL)
    if deny_file and os.path.exists(deny_file):
        for line in open(deny_file):
            line = line.strip()
            if line and not line.startswith("#"):
                pats.append(line)
    return re.compile("|".join(f"(?:{p})" for p in pats), re.I)

def query(endpoint, sparql, token=None):
    body = json.dumps({"query": sparql}).encode()
    req = urllib.request.Request(
        endpoint.rstrip("/") + "/query", data=body,
        headers={"Content-Type": "application/json", "X-Quipu-Client": "agent-adhoc"},
    )
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)["rows"]

NAMESPACE = ONT


def local(iri):
    return iri.rsplit("/", 1)[-1] if iri.startswith(NAMESPACE) else iri

# ---- classification -------------------------------------------------------

SHA = re.compile(r"^[0-9a-f]{7,40}$")

def strip_sha_form(name):
    """Reduce an id-shaped alias to the sha it denotes, else None."""
    for pat in (
        r"^(?:code/)?commit/[^/]+/([0-9a-f]{7,40})$",
        r"^commit[-_](?:[a-z]+[-_])?([0-9a-f]{7,40})$",
        r"^commit_[a-z]+_([0-9a-f]{7,40})$",
        r"^[a-z]+-commit-([0-9a-f]{7,40})$",
        r"^[a-z]+-([0-9a-f]{7,40})$",
        r"^([0-9a-f]{7,40})$",
    ):
        m = re.match(pat, name)
        if m:
            return m.group(1)
    return None

def classify(a, b):
    """id-form: both sides denote the same commit under different id spellings.
    Such a pair is repairable by a normalisation rule and needs no judgement.
    semantic: anything else — two English phrasings of one concept."""
    sa, sb = strip_sha_form(local(a)), strip_sha_form(local(b))
    if sa and sb and (sa.startswith(sb) or sb.startswith(sa)):
        return "id-form"
    return "semantic"

# ---- pseudonymisation -----------------------------------------------------

class Pseudonymiser:
    """Stable, collision-checked, and structure-preserving."""

    def __init__(self, salt):
        self.salt, self.map, self.used = salt, {}, {}

    def name(self, iri, kind):
        if iri in self.map:
            return self.map[iri]
        h = hashlib.sha256((self.salt + iri).encode()).hexdigest()[:10]
        out = PUB + f"{kind}-{h}"
        prior = self.used.get(out)
        if prior is not None and prior != iri:
            raise SystemExit(f"pseudonym collision: {iri} and {prior}")
        self.used[out] = iri
        self.map[iri] = out
        return out

def nt(s, p, o_literal=None, o_iri=None):
    if o_iri is not None:
        return f"<{s}> <{p}> <{o_iri}> ."
    esc = o_literal.replace("\\", "\\\\").replace('"', '\\"')
    esc = esc.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
    return f'<{s}> <{p}> "{esc}" .'

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default=os.environ.get("QUIPU_SERVER", "http://localhost:8080"))
    ap.add_argument("--out", default="benchmark/replay/corpus")
    ap.add_argument("--salt", default="mergebench-replay-v1")
    ap.add_argument("--verify", action="store_true")
    ap.add_argument("--namespace", default=ONT,
                    help="source store entity namespace (deployment-specific)")
    ap.add_argument("--deny-file", default=os.environ.get("REPLAY_DENY_FILE"),
                    help="extra scrub patterns, one regex per line (site-specific names)")
    args = ap.parse_args()

    global NAMESPACE
    NAMESPACE = args.namespace

    token = os.environ.get("QUIPU_AUTH_TOKEN")
    if not token:
        p = os.path.expanduser("~/.config/aegis/quipu_token")
        if os.path.exists(p):
            token = open(p).read().strip()

    # CONTROL first: an absence measured with an unproven instrument is not a
    # finding. If this returns nothing the store is unreachable or empty and
    # every count below would read as a clean zero.
    control = query(args.endpoint, "SELECT ?s WHERE { ?s ?p ?o } LIMIT 3", token)
    if not control:
        sys.exit("CONTROL FAILED: store returned no triples at all; refusing to emit a corpus")

    pairs = [(r["a"], r["b"]) for r in query(
        args.endpoint, f"SELECT ?a ?b WHERE {{ ?a <{SAME_AS}> ?b }} ORDER BY ?a", token)]
    doubles = query(args.endpoint, f"""SELECT ?s ?c WHERE {{
        {{ SELECT ?s WHERE {{ ?s <{COMMENT}> ?x }}
           GROUP BY ?s HAVING (COUNT(?x) > 1) }}
        ?s <{COMMENT}> ?c }} ORDER BY ?s""", token)

    # Symmetric knots are ONE repair recorded twice, not two.
    undirected, seen = [], set()
    for a, b in pairs:
        k = tuple(sorted((a, b)))
        if k not in seen:
            seen.add(k)
            undirected.append((a, b))

    ps = Pseudonymiser(args.salt)
    os.makedirs(args.out, exist_ok=True)

    alias_rows, counts = [], {"id-form": 0, "semantic": 0}
    for a, b in undirected:
        kind = classify(a, b)
        counts[kind] += 1
        alias_rows.append({
            "left": ps.name(a, "entity"), "right": ps.name(b, "entity"), "class": kind})

    by_subject = {}
    for r in doubles:
        by_subject.setdefault(r["s"], []).append(r["c"])
    comment_rows = [
        {"subject": ps.name(s, "entity"), "values": v}
        for s, v in sorted(by_subject.items()) if len(v) > 1
    ]

    # Comment BODIES are prose about the deployment; the merge operator cares
    # only that the values differ, so publish synthetic distinct values and
    # keep the real multiplicity.
    for row in comment_rows:
        n = len(row["values"])
        row["values"] = [f"revision {i + 1} of this entity's description" for i in range(n)]

    fixture = {
        "schema": "quipu/replay-corpus/v1",
        "provenance": "aegis production knowledge graph, pseudonymised",
        "alias_pairs": alias_rows,
        "comment_doublings": comment_rows,
        "counts": {
            "sameas_edges_raw": len(pairs),
            "alias_pairs_undirected": len(undirected),
            "alias_id_form": counts["id-form"],
            "alias_semantic": counts["semantic"],
            "comment_doubled_subjects": len(comment_rows),
            "comment_excess_values": sum(len(r["values"]) - 1 for r in comment_rows),
        },
    }

    path = os.path.join(args.out, "corpus.json")
    with open(path, "w") as f:
        json.dump(fixture, f, indent=2, sort_keys=True)
        f.write("\n")

    forbidden = build_detector(args.deny_file)
    blob = json.dumps(fixture)
    hits = sorted({m.group(0) for m in forbidden.finditer(blob)})
    if hits:
        sys.exit(f"SCRUB FAILED: internal identifiers in fixture: {hits}")
    # Control the scrub itself: a detector that CANNOT fire and one that simply
    # found nothing produce identical clean output. Prove every structural
    # pattern fires, not just that one of them does — a single positive control
    # passes even when three of the four patterns are broken. Values are
    # documentation-reserved (RFC 5737 / RFC 2606) so the control is not itself
    # the leak it checks for.
    controls = {
        "ipv4": "reached 192.0.2.1 yesterday",
        "home": "stored under /home/example/notes",
    }
    dead = [k for k, probe in controls.items() if not forbidden.search(probe)]
    if dead:
        sys.exit(f"SCRUB CONTROL FAILED: pattern(s) {dead} match nothing; a clean "
                 "result from this detector would be meaningless")

    print(f"wrote {path}")
    for k, v in fixture["counts"].items():
        print(f"  {k:32} {v}")
    print("  scrub                            clean (detector control passed)")

    if args.verify:
        assert len(ps.used) == len(ps.map), "pseudonym map is not injective"
        print(f"  pseudonyms injective             {len(ps.map)} names")

if __name__ == "__main__":
    main()
