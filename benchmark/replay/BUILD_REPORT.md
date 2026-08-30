# ARM B / ARM C — recorded divergence replayed against the shipped operator

Read this before quoting a number from `cargo run --example replay --features shacl`.

## What this arm is for

ARM A (`mergebench`) scores eight merge strategies on a *synthetic* corpus, and its
own build report says the shape-aware arm's precision there is **definitional**: the
oracle's rules and the operator's coincide by construction. A synthetic arm can
therefore measure what the alternatives *cost*, but it cannot be evidence that our
operator is *correct*.

This arm exists to remove that circularity twice over:

1. the corpus is **recorded production divergence**, not generated; and
2. the operator under test is the **shipped** `quipu::share_merge`, driven through
   real share bundles built by `quipu::share` — not the benchmark's reimplementation.

Here a disagreement between operator and expectation is a finding, not a bug in the
harness's copy of the rules.

## The corpus

Built by `scripts/build-replay-corpus.py` from the live aegis knowledge graph,
anonymised (see the limits section — the first anonymisation was reversible), and
committed at `benchmark/replay/corpus/corpus.json`.

| quantity | value |
|---|---|
| `owl:sameAs` edges recorded | 171 |
| distinct undirected repairs | **105** (66 edges were one knot recorded twice) |
| — id-form (short sha ↔ `commit/<repo>/<full-sha>`) | 52 |
| — semantic (two phrasings of one concept) | 53 |
| subjects with a doubled `rdfs:comment` | **939** |
| excess comment values | **1,784** |

Each `owl:sameAs` knot is one repair a person actually performed, so the 105 is the
manual-effort baseline this arm is measured against.

**The alias class splits, and the split matters.** Roughly half the recorded
repairs (52/105) are *id-form*: two spellings of the same commit id. Those are
mechanically normalisable — a rule could retire them without asking anyone. The
other 53 are two different English phrasings of one concept, where nothing but a
person can decide. Reporting 105 as one undifferentiated "human decisions" number
would overstate the irreducible cost by about a factor of two.

## Results

```
scenario         decisions historical   outcome
alias-id-form            0         46    merged     FALSE NEGATIVE
alias-semantic           0         47    merged     FALSE NEGATIVE
comment-double         939          0 conflicts     raised as decisions
sameas-repair            0         20    merged     repair survives (wanted)
```

- **The operator is blind to the entire alias class — 0 of 93.** This reproduces
  mergebench's synthetic 0/4 on a real corpus, and it is the honest false-negative
  column: two names for one entity is invisible to *any* triple-level operator,
  shape-aware included. No amount of schema fixes this; it is a limit of the medium
  the operator works in.
- **The operator raises 939 decisions on the class production surfaced 0 of.** The
  same class produced 1,784 silent excess comments through the append path.
- **A repair survives a concurrent edit.** `owl:sameAs` is unconstrained, so a knot
  made on one side unions rather than racing an edit on the other. A merge that
  could drop repairs would make the repair path itself unreliable.

12 pairs (6 id-form, 6 semantic) were excluded as **chained** — an endpoint shared
with another pair, e.g. one entity knotted both to `Quipu` and to `quipu-repo-github`.
Including them would have made the arm measure an incidental collision on the shared
endpoint rather than alias detection; the first run of this harness did exactly that
and reported 2 and 1 spurious decisions. Alias chains are a real repair-ordering
hazard and are reported rather than dropped.

## The negative control

`--negative-control` reruns everything with `rdfs:comment` unbound:

```
comment-double           0          0    merged
```

939 → 0. The decisions are therefore **shape-derived**, not manufactured by the
harness. This control arm is also the closest thing to a simulation of production's
append path, and it reproduces the doubling — which is the paper's thesis stated as
an experiment: the schema defines the conflict.

`--selftest` asserts both directions (detector fires on the functional class, stays
silent on the alias class) and exits non-zero on failure.

## ARM C — share / diverge / reconnect

`--case-study` walks the lifecycle on a real bundle and verifies every hop:

```
1. SHARE base bundle          share_id sha256:…  graph_hash sha256:476d6…
2. DIVERGE peer publishes     parent = the base share_id
3. DIVERGE we edit locally
4. STATUS before reconnect    diverged=true  ours+40 theirs+40  conflicts 0
5. RECONNECT merge            outcome merged  asserted 40  retracted 0
6. PROVENANCE two parents     recorded on the transaction
7. CONVERGED both sides kept  ours 40/40  theirs 40/40
8. STATUS after reconnect     outstanding from theirs 0
```

`graph_hash` is stable across runs; `share_id` is not, because the manifest carries
a timestamp. Fork-point anchoring depends on the graph hash, so this does not affect
lineage — but do not use `share_id` as a content identity.

## What this arm does NOT establish

- **The 939 is a counterfactual, and must be labelled as one in the paper.** The
  real doublings arose from *sequential re-posts through a single write path*, not
  from a share/diverge/merge. The claim this supports is narrow: had the same edits
  arrived as a divergence and been reconciled by this operator, each would have been
  surfaced as a decision instead of silently doubling. It is not a claim that the
  operator was running and prevented 1,784 doublings.
- **The comment bodies are synthetic.** Real ones are prose about the deployment.
  Only the *multiplicity* is real, which is all the merge reasons about.
- **The corpus is anonymised, and it was not always.** Structure, cardinality and
  class membership are preserved exactly; injectivity is verified over 1,076
  names, every one matching `entity-<hex>`.

  This bullet used to end "so no source identifier can survive by construction,
  which is a stronger guarantee than the extractor's own scrub gate." **That was
  false, and it was false in the reassuring direction.** Names were
  `sha256(salt + iri)[:10]` with the salt defaulting to a literal committed in
  the public extractor and the source namespace appearing in 66 files of this
  repository. Both inputs to the digest therefore shipped with the artifact, so
  any candidate IRI could be *confirmed* by recomputing the digest and looking
  for it here. A ~60-word hand-typed wordlist recovered 41 real entity names —
  host names, service names, the full contributor roster.

  Two things are worth keeping from that. First, "injective over 1,076 names,
  every one matching `entity-<hex>`" was **true** and was offered as the evidence
  for a claim it does not support: a bijection to opaque-looking labels says
  nothing about whether the map can be inverted. Second, no pattern-matching
  scrub gate could ever have caught it, because there is no forbidden string in
  the file — which is why the claim survived one.

  Names are now drawn from a CSPRNG and the mapping is discarded at build time,
  so there is no preimage to recover rather than a key to keep. `python3
  scripts/reseal-replay-corpus.py --check` re-runs the recovery attempt and is
  wired into `scripts/arxiv-scrub-gate.sh`; it proves its own oracle against a
  known-leaking fixture first, so a clean result is not merely a silent one.
- **A conflict blocks the whole merge, not the conflicting slot.** `merge` returns
  `outcome: conflicts` with `asserted: 0` and writes nothing at all. On the real
  939-subject slice that means a single reconnect would write nothing until every
  one of them is resolved. Whether that all-or-nothing choice is right is a design
  question the paper should raise, not one this arm settles.
- **Decision *quality* is unmeasured.** This arm counts decisions demanded; it does
  not evaluate whether a person would have found each one worth being asked.

## Reproducing

The extractor keeps only deployment-neutral patterns (bare IPv4, home paths) in
its default scrub list; site conventions such as private-network suffixes are
supplied at run time with `--deny-file` / `REPLAY_DENY_FILE`. An earlier revision
inlined them, and this repository's pre-push scrub guard refused the push naming
the extractor's own *control probes* as the leak. It was right to — a guard cannot
distinguish a real name from an example of one, and splitting the literal to get
past it would have hidden the string from the next reader's grep as well. The fix
was for the public file not to need the strings at all.

```bash
export REPLAY_NAMESPACE=...                         # source store namespace
python3 scripts/build-replay-corpus.py --verify     # needs a live store; scrubs + controls itself
cargo run --example replay --features shacl
cargo run --example replay --features shacl -- --selftest
cargo run --example replay --features shacl -- --negative-control
cargo run --example replay --features shacl -- --case-study
```

The committed corpus is sufficient for all four; the extractor is only needed to
rebuild it.
