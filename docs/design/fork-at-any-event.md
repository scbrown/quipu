# Design: Fork-at-any-event — persistent named forks

> **Implementation status (2026-08-22):** ✅ **v1 complete** (quipu-gp5).
> The `forks` registry table, `Store::fork_create/fork_list/fork_lookup/
> fork_diff/fork_drop/fork_promote` (`src/store/forks.rs`), the `quipu fork`
> CLI (`src/cli_fork.rs`), `--fork <name>` on `quipu read`, and a `fork`
> parameter on the MCP/REST `/query` context (`src/mcp/mod.rs::query_context`)
> all shipped together, with the meta-graph mirror, `fork.*` registry events,
> and respace classification. Verified by 8 store tests plus the respace
> classification suite; full lib suite green.

**Status:** the ActiveGraph comparison (arXiv:2605.21997) demonstrated
fork→test→promote as a one-liner UX. Quipu already held the pieces —
`Store::speculate` is closure-scoped and uncommitted, and bitemporal replay
reads any past state — but had no *persistent, named* fork. This adds one,
under the constraint the bead states up front: **fork ergonomics must never
become a gate bypass.** The evidence bar is the thing ActiveGraph does not
have, and we do not give it up.

## 1. What a fork is

A fork is a **committed-class named graph** (`urn:quipu:fork:<name>`) pinned
to a parent transaction, plus one row in a `forks` registry table
(`name, g, parent_branch, fork_tx, created_at, status`). There are **no
storage-engine changes**: creating a fork materializes ROOT's
live-as-of-`fork_tx` triples into the fork's graph under one new transaction,
and from then on the fork is an ordinary named graph — an independent lineage
that ROOT's later history cannot touch.

The as-of snapshot uses the same predicate the SPARQL `as_of_tx` path uses
(quipu #83): `op = 1 AND tx <= N AND (valid_to IS NULL OR retracted_tx > N)`.
That identity is the acceptance criterion: *querying the fork* and *querying
ROOT as of N* return the same triples. A row live at N but retracted since is
copied **open** (`valid_to NULL`) — in the fork's lineage that retraction
never happened.

Because the fork graph is a registered committed graph, the existing strict
`graph` parameter on `/knot` already reaches it: that is the sanctioned v1
way to write *into* a fork (as is `transact_to_graph` from the library).

## 2. Reading a fork

- CLI: `quipu read "<sparql>" --fork <name>`.
- MCP/REST query context: `{"fork": "<name>"}` — mutually exclusive with
  `graph`; one request, one scope authority.
- Both resolve the *name* through the registry to
  `GraphScope::Default([fork_g])`. Unknown and **dropped** forks are refused
  loudly — never a silent fall-through to ROOT. SPARQL already composes graph
  scope with `valid_at`/`as_of_tx`, so time-travel inside a fork needs no
  evaluator changes.

## 3. Diff

`quipu fork diff <a> <b>` — each side a fork name or `main`/`ROOT` —
compares the two graphs' **present-state `(e, a, v)` sets** (term-id
comparison is sound because both sides live in one store's dictionary) and
prints `+` (in `b` only) / `-` (in `a` only) lines, lexically sorted.

Scope, stated plainly: present-state triple sets only. **No
valid-time-interval diff and no per-transaction attribution** — those
questions belong to `unravel` and the event log, and pretending a set
difference answers them would be worse than not answering.

## 4. Drop

`quipu fork drop <name>` sets `status = 'dropped'` and emits `fork.dropped`.
The graph's facts and the meta-graph mirror are **left in place** — the
`dataset_remove` precedent: a fork that existed is a fact about the past.
Consequences: a dropped fork's name is not reusable, and every fork operation
(read-by-name, diff, promote, drop) refuses it thereafter.

## 5. Promotion routes through the gates — why

`quipu fork promote <name>` computes the structural delta fork-vs-ROOT and
applies it to ROOT as ordinary assert/retract datums. Two layers gate it:

1. **SHACL first, at the promote call site.** The asserted delta is
   serialized to N-Triples and validated against the stored shapes'
   **reject-mode** half (`split_shapes_by_policy` — emit-mode shapes observe,
   they do not gate), with the store as repair context
   (`validate_with_store_context`). Non-conformance returns a real refusal:
   verdict details reported, **nothing written**, the fork stays `open` for
   repair.
2. **Then `transact_to_graph(..., ROOT)`**, which supplies the authority,
   placement, policy and OWL gates every ROOT write faces. An `Err` from any
   of them rolls the whole promotion (including the status flip) back.

The SHACL check is deliberately **not** inside `transact_to_graph` — that
would change the contract of every caller. It lives at the promote call site,
exactly like `/knot`'s. And it is deliberately not skippable: a fork is the
ergonomic place to accumulate speculative facts, which is exactly why its
exit must present the same evidence bar as any other write. Limits worth
stating: SHACL validates the *asserted* delta (a retraction that strips
context out from under surviving ROOT facts is the policy/OWL gates' domain,
same as any other retraction), and promotion stamps the delta with the fork
facts' own `valid_from` claims, not the promote instant.

## 6. Registry, provenance, events

- Creation mirrors the fork into the meta-graph in the same savepoint
  (`quipu:Fork` typing + `quipu:forkTx` pin), through the ordinary write
  gates — the `dataset_create` precedent — so forks are queryable and
  governable like any other registry object.
- `fork.created` / `fork.promoted` / `fork.dropped` ride the same registry
  event spine as `shapes.loaded` (`emit_registry_event`), so the audit spine
  records that a lineage was opened, landed, or abandoned.
- `respace` classification: `forks.g` and `forks.parent_branch` carry term
  identity (`Id`); `fork_tx` is a **transaction** id — an integer that looks
  exactly like a term id and is not.

## 7. The legacy-rows caveat (inherited from quipu #83)

Rows closed before quipu #83 have `valid_to` set and `retracted_tx` NULL, so
the as-of predicate leaves them invisible at **every** N — the transaction
that closed them was never recorded, and guessing would place them in windows
they may not have been live in. A fork of old history therefore **can
under-report**: facts that were genuinely live at N but were retracted before
the #83 migration will be missing from the fork. This is stated, not hidden;
it is the same honest gap every `as_of_tx` read has.

## 8. Explicitly out of v1, and why

- **Nested forks (fork of a fork).** `parent_branch` is stored precisely so
  this can grow later, but v1 forks only ROOT — one level keeps
  promote-target resolution trivial and un-ambiguous.
- **Per-fork shape sets.** The stored shapes are policy about ROOT; a fork
  that wants different rules is a different governance question, not a
  storage question.
- **Reasoner runs scoped to a fork.** The reasoner reads and writes one
  selected graph already, but wiring its CLI/config to fork names is untested
  surface; do it when a consumer needs it.
- **`--fork` on mutation endpoints.** Writes into a fork go through `/knot`'s
  strict `graph` parameter or `transact_to_graph`; a `--fork` write
  convenience can come later without changing semantics.
- **Valid-time-aware diff / per-tx attribution.** See §3.
- **Fork-as-separate-file.** `quipu graph import` / attachments already cover
  the cross-store case; a fork is deliberately *in-store* so diff and promote
  stay one-dictionary operations.
- **A full MCP fork-management surface.** v1 mirrors only the read path (the
  `fork` query param). Create/diff/drop/promote over MCP is follow-up work —
  file it when an agent consumer materializes.
