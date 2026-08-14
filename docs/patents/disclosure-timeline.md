# Public Disclosure Timeline — Patentability Assessment

**Status:** rev 2, 2026-08-14 — corrected after independent adversarial
re-derivation of every row across all seven sibling repositories. Not legal
advice; prepared to support provisional filings and attorney review.

## Purpose

US patent law (35 U.S.C. §102(b)(1)) gives an inventor a one-year grace
period from their **own** first public disclosure of an invention. Every
mechanism below was first disclosed in a public GitHub repository, so each
has its own one-year filing deadline. This document reconstructs those dates
so the provisional applications are filed inside every window.

Absolute-novelty jurisdictions (EP-style) are foreclosed by these
disclosures. Jurisdictions with their own 12-month inventor grace periods
(JP, KR, CA, AU) are *potentially* still open on the same clocks as the US —
attorney advice needed per jurisdiction. This timeline computes US deadlines.

## Method and assumptions

- Dates are **git commit author dates** on the public repositories, taken as
  a proxy for public disclosure. Frequent tagged releases corroborate them
  (bobbin v0.1.0 2026-02-09 → v0.6.7 2026-08-07; quipu v0.1.0 2026-04-04 →
  v0.3.23 2026-08-12; both verified against `git for-each-ref`).
- **Merge-commit dates can UNDERSTATE disclosure**: several mechanisms landed
  via squash/merge commits whose content was pushed on public PR branches —
  and discussed in public PRs — before the main-branch date. That shortens a
  window (the dangerous direction). Where a row cites a merge commit, treat
  the date as a latest-bound and check the PR branch push date before
  relying on it.
- Where a design document predates code, the **earlier** date is used — a
  public enabling description is a disclosure whether or not code exists.
- **Sibling repos disclose each other's mechanisms.** All seven repos
  (quipu, bobbin, yupana, shantytown, camayoc, NeuralAmplifier, creel) were
  searched; one cluster-A mechanism was first disclosed in yupana, not
  quipu. Cross-repo rows are marked.
- Every citation in this revision was independently re-derived after three
  citations in rev 1 were found to be false pickaxe hits (matches inside
  minified vendor JS, incidental prose, and a third-party test fixture).

## Provisional A — quipu governance cluster

| Mechanism | First disclosure | Evidence | US filing deadline |
|---|---|---|---|
| Evidence-hash-bound ed25519 verdict signing | 2026-07-19 | quipu `2336fc7` (`src/signing.rs`), `325630c` (`evidence_hash`); yupana's mirror is later (07-22, `bfe144e`) | **2027-07-19** |
| Content-bound escalation router; expiry/auto-reject | **2026-08-01** | **yupana** `baae03e`, `docs/book/src/design/governance-plane.md`: `DecisionRequest`, content-bound approval of hash H, bounded waits with auto-reject — pre-discloses the mechanism. Precursors: quipu `325630c` 07-19 (escalate effect), yupana `e562027` 07-20, quipu `a0bddd7` 07-21 (Q-APPROVAL). Conservative reading: 2026-07-21 | **2027-08-01** (conservatively 2027-07-21) |
| Verdict staging + post-savepoint flush; re-entrancy carve-out | 2026-08-03 | quipu `5e7a316` (`flush_pending_verdicts`, `recording_verdicts`) | 2027-08-03 |
| Monotone two-pass contextual SHACL | 2026-08-05 | quipu `7e29558` (`src/shacl_context.rs`) | 2027-08-05 |
| Label lattice (fallible chain-scoped trust meet) | 2026-08-06 | quipu `7504202` (`src/lattice.rs`) | 2027-08-06 |
| Coverage-as-fold-identity | 2026-08-06 | quipu `7504202` (`Coverage::Empty`) — rev 1 cited `1a83e48`, a false hit in vendored three.js | 2027-08-06 |
| Graph labels + bitemporal de-declaration | 2026-08-06 | quipu `94b6111`, `9ff3dde` | 2027-08-06 |
| Bitemporal shape versioning, as-of validation | 2026-08-06 | quipu `4897d98` | 2027-08-06 |
| Audit replay fidelity/drift split | 2026-08-06 | quipu `531618e` (`replay_as_of`) | 2027-08-06 |

**Controlling statutory deadline for cluster A: 2027-07-19** (verdict
signing), with the escalation-router row conservatively at **2027-07-21**.

## Provisional B — bobbin retrieval cluster

| Mechanism | First disclosure | Evidence | US filing deadline |
|---|---|---|---|
| Coupling expansion (generic co-change retrieval expansion) | **2026-01-02 – 02-07** | coupling table + `get_coupling` in the 01-02 scaffolding (`8a449ee`, `2acfb57`); co-change algorithm in initial beads export (`d34c8a5`); "coupling expansion depth" in context assembly (`f30c983`/`d0b15ea`, 02-07) | **2027-01-02** for broad claims |
| Abstain gate on pre-fusion cosine | 2026-02-08 | `6ea9b0b` (plan: gate on raw cosine captured before RRF) + `c07d865` (impl, same day) | **2027-02-08** |
| Session-aware injection dedup (ledger precursor) | **2026-02-08** | `673e9cd`: session ID from top-10 chunk keys, skip-if-unchanged, per-chunk injection frequencies — the 03-05 "progressive reducing" ledger (`1831e0e`) is a refinement of this | **2027-02-08** |
| Blame-provenance bridging | 2026-02-11 | `c53ea64` (`bridge_docs_via_provenance()` via `git blame -L --porcelain`); code 02-26 (`5686e1a`) | 2027-02-11 |
| Failure-triggered parse-directed retrieval | 2026-02-15 | `6c2c664` — rev 1 cited `dc21343` (02-09), a false hit in a third-party tool's test fixture | 2027-02-15 |
| Asymmetric query preprocessing | 2026-02-18 | `164751e` (`src/search/preprocess.rs`) | 2027-02-18 |
| Retrieval-parameter calibration (eval-based) | 2026-02-18 | `2a981c3` (`eval/runner/calibrate.py` grid sweep) — governs broad calibration claims | 2027-02-18 |
| Self-supervised calibration from repo history | 2026-02-25 | `1753e04` (`src/cli/calibrate.rs`: commits-as-queries, changed-files-as-ground-truth) — the distinct commit-history-supervised mechanism | 2027-02-25 |
| Feedback storage / rating loop | 2026-03-02 | `aa1f6e5` (`src/storage/feedback.rs`) | 2027-03-02 |
| Intent-conditioned retrieval parameters | 2026-03-05 | `ba5c035` (`src/search/intent.rs`) | 2027-03-05 |
| Complementary (unseen-files) expansion | 2026-03-22 | `0aed890` — rev 1 cited `02817c0` (02-08), incidental prose about lint tools | 2027-03-22 |
| Query-overlap-weighted feedback propagation | 2026-03-22 | `91fe8b1` (`file_feedback_scores`) | 2027-03-22 |
| Cross-repo coupling via issue-ID trailers | 2026-06-27 | `0edae39` (merge commit — PR branch pushed earlier; see Method). Broad "cross-repo relatedness" was disclosed 02-07 (`d0b15ea` `--cross-repo` mode) and 02-14 (`965956b`) | 2027-06-27 narrow; **2027-02-07** broad |
| **Combination (closed-loop injection controller)** | 2026-01-02 – 02-08 | inherits its oldest element — coupling expansion | **2027-01-02** |

**Controlling deadline for cluster B: treat as 2027-01-02.** Individual
narrow mechanisms hold their own later dates, but any claim reciting
coupling expansion as an element — including the combination claim —
inherits the January date.

## Provisional C — NeuralAmplifier decision-delegation cluster

Repository history begins 2026-07-26; the youngest cluster by a wide margin.

| Mechanism | First disclosure | Evidence | US filing deadline |
|---|---|---|---|
| World-view / action-space / deadline contract | 2026-07-26 | `cf47912` (`docs/contract.md`) | **2027-07-26** |
| Fallback-gated per-surface tiering; frozen registry; NO_AI_PATH | 2026-07-29 | `ee00e99` (`surfaces.py`) | 2027-07-29 |
| Derived fairness ledger with drift detection | 2026-07-29 | `1b7f245` (`handicap_drift`) | 2027-07-29 |
| Traceparent-correlated outcome accounting | 2026-07-29 | `f05f032` (observability design + contract telemetry fields) | 2027-07-29 |
| Grounding-utilisation unmeasurable-not-zero | 2026-07-29 | `87471f0` | 2027-07-29 |
| Directives (closed vocabulary, `unmeasurable` status, `setback_turns`) | 2026-07-30 | `2cc9b30`; `directives.py` added same day | 2027-07-30 |
| Agent-as-brain MCP; injection-safe doorbell | 2026-07-31 | `f471f95` (doorbell); `agent_brain.py` added same day | 2027-07-31 |
| Two-clock deadline margin; `run_id` generation fencing | 2026-08-01 | `9cc3efc` (`decision_deadline_ms`), `b13c0f2` (retire pendings of a dead game process) | 2027-08-01 |
| Two-door command channel (4 Hz poll, consume-before-act) | 2026-08-02 | `1b1a4a7` (`docs/turn-scoped-play.md`) | 2027-08-02 |

**Controlling statutory deadline for cluster C: 2027-07-26** (the contract
disclosure; any combination claim inherits it). All C citations were
spot-checked against commit content after the rev-1 false-hit lesson; the
07-29 `unmeasurable` hit was verified to concern grounding utilisation (its
own row), not the directive status, which correctly dates to 07-30.

## Practical deadline

Both provisionals should be **filed by 2026-08-31** — the inventor begins
employment 2026-09-01 under an agreement whose invention-assignment clause
may reach future filings. Filing before the start date, and listing both
applications and all seven repositories on the employer's prior-inventions
exhibit, establishes these as pre-existing inventions. An August filing also
moots every shortened window identified above, which makes this the
load-bearing deadline of the whole exercise.

## Caveats

- **PR-branch timing**: commit author dates on main can postdate the true
  public push (see Method). For any row within a month of its deadline,
  re-derive against branch push and PR dates before relying on it.
- **AI-authored disclosing commits**: many disclosing commits are authored
  or co-authored by AI agents. §102(b)(1)(A) shelters disclosures by the
  inventor or by another who obtained the subject matter from the inventor;
  the human-inventor derivation chain behind agent-authored public
  disclosures should be confirmed with the attorney alongside the
  inventorship analysis for the claims themselves.
- **Repo visibility dates** (when each repo became public) are assumed to
  predate the cited commits; this was not independently evidenced. That
  error direction only extends deadlines.
- Micro entity vs small entity: because the inventor has signed an
  employment agreement that may create an assignment obligation, **small
  entity** certification is the safe choice; undercertification risks
  unenforceability, overpayment carries no penalty.
