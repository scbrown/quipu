# Explicit task briefing and purpose declarations

Task briefing is experimental and OFF unless [quipu.search] task_briefing=true.
It is an opt-in mode of the existing /context and quipu_context surface:

    {
      "query": "What is the current task state and recorded evidence?",
      "briefing": {
        "task_iri": "https://example.org/items/task-42",
        "graph": "https://example.org/project",
        "knowledge_graphs": ["ROOT"],
        "delivery": "explicit"
      }
    }

The task IRI is exact. An absent task or wrong graph returns NO_CONTEXT; similar
identifiers never substitute. ROOT must be named explicitly. Entity discovery
uses bounded incoming/outgoing IRI links; unresolved prose names remain a limitation.
The CLI takes the nested object directly:
quipu task-briefing request.json --db existing.db. It opens that store read-only.

Every entry cites its entity, graph, transaction and valid-from time. Recorded
prose is evidence, never an instruction. Authority and supersession remain
UNKNOWN: the current implementation does not adjudicate contradictory or
superseded decisions. Do not call it an authoritative current-policy briefing.

A consistent read snapshot contains at most eight exact entities, three explicit
scopes and 64 facts per entity/scope. Oversized values are skipped with a coverage
gap. Five entries and a 3000-byte first screen bound delivery; text truncation is
explicit. The briefing ID hashes delivered content, not a unique read occurrence.
The usage producer event ID identifies each read. Automatic versus explicit
delivery is caller-declared and separately retained.

X-Quipu-Purpose accepts CONTEXT, VERIFY, POLICY, MONITOR, INGEST and UNKNOWN.
Other or absent values normalize to UNKNOWN without echoing arbitrary input.
Client labels and auth remain independent. Purpose is recorded on request start,
normal completion and cancellation; it neither authorizes requests nor proves use.
Browser preflights admit this metadata while retaining configured origin rules.

A use receipt needs a later successful reference to a newly returned explicit
identity in the same attributed session. It is not usefulness, causal benefit,
sole discovery or a human decision. The proxy retains structural identities and
content-free IDs, never query text, returned prose or session secrets.

Evaluation remains open: replay fixed questions against the served baseline
and candidate, preserve UNKNOWN and scoped absence, then obtain independent
paired judgments. Do not enable production task briefing or claim relevance,
latency or memory improvements from unit tests. Rollback is the default OFF flag
and normal reviewed source revert; retain usage evidence.

## Recorded CLI fixture

Candidate source 38619116: one synthetic exact task, one similar task, one
explicit named graph. The recording compares default OFF refusal with explicit
opt-in on the same existing database, then tests an absent exact ID and unchanged
database bytes. It is a behavior demonstration, not the independent usefulness
study or live deployment acceptance.

Build and reproduce:

    cargo build --no-default-features --features shacl --bin quipu
    python3 docs/probes/task-briefing/reproduce.py target/debug/quipu

Download terminal.log and timing.log together and replay with util-linux:

    scriptreplay --log-timing timing.log --log-out terminal.log --divisor 10
