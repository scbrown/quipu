# Public Disclosure Timeline — Patentability Assessment

**Status:** working document, 2026-08-14. Not legal advice; prepared to support
provisional filings and attorney review.

## Purpose

US patent law (35 U.S.C. §102(b)(1)) gives an inventor a one-year grace period
from their **own** first public disclosure of an invention. Every mechanism
below was first disclosed in a public GitHub repository, so each has its own
one-year filing deadline. This document reconstructs those dates so the
provisional applications are filed inside every window.

Jurisdictions requiring absolute novelty (EP, JP, CN, and most others) are
already foreclosed by these disclosures. This timeline is US-only.

## Method and assumptions

- Dates are **git commit author dates** on the public repositories
  (`github.com/scbrown/quipu`, `github.com/scbrown/bobbin`), taken as a proxy
  for public disclosure. Both repos push promptly and cut frequent tagged
  releases (bobbin v0.1.0 on 2026-02-09 through v0.6.7 on 2026-08-07; quipu
  v0.1.0 on 2026-04-04 through v0.3.23 on 2026-08-12), which corroborates the
  commit dates within days. crates.io publish dates and the Zenodo DOI give
  further independently timestamped disclosures if needed.
- Where a design document predates the implementing code, the **earlier** date
  is used — a public enabling description is a disclosure whether or not code
  exists yet.
- Dates were located with `git log -S<symbol> --reverse` (first commit
  introducing the mechanism's distinguishing symbol) and `git log --follow
  --reverse` on the implementing files, on full (unshallowed) history.

## Provisional A — quipu governance cluster

Repository history begins 2026-04-04. All mechanisms are recent.

| Mechanism | First disclosure | Evidence | US filing deadline |
|---|---|---|---|
| Evidence-hash-bound ed25519 verdict signing | 2026-07-19 | `src/signing.rs` added; `evidence_hash` introduced (`325630c`) | **2027-07-19** |
| Verdict staging + post-savepoint flush; re-entrancy carve-out | 2026-08-03 | `flush_pending_verdicts`, `recording_verdicts` (`5e7a316`) | 2027-08-03 |
| Content-bound escalation router; expiry-as-denial | 2026-08-03 | `DecisionRequest` (`0ac14bc`), `Ruling` (`fbf8be7`) | 2027-08-03 |
| Coverage-as-fold-identity | 2026-08-03 | `Coverage::Empty` (`1a83e48`) | 2027-08-03 |
| Monotone two-pass contextual SHACL | 2026-08-05 | `src/shacl_context.rs` added (`7e29558`) | 2027-08-05 |
| Label lattice (fallible chain-scoped trust meet) | 2026-08-06 | `src/lattice.rs` added (`7504202`) | 2027-08-06 |
| Graph labels + bitemporal de-declaration | 2026-08-06 | `docs/design/graph-labels.md` (`94b6111`) | 2027-08-06 |
| Bitemporal shape versioning, as-of validation | 2026-08-06 | `get_combined_shapes_as_of` (`4897d98`) | 2027-08-06 |
| Audit replay fidelity/drift split | 2026-08-06 | `replay_as_of` (`531618e`) | 2027-08-06 |

**Controlling statutory deadline for cluster A: 2027-07-19** (earliest
mechanism: evidence-hash verdict signing).

## Provisional B — bobbin retrieval cluster

Repository history begins 2026-01-02.

| Mechanism | First disclosure | Evidence | US filing deadline |
|---|---|---|---|
| Abstain gate on pre-fusion cosine | 2026-02-08 | `gate_threshold` (`6ea9b0b`); `docs/plans/smart-hook-injection.md` same day | **2027-02-08** |
| Complementary coupling expansion | 2026-02-08 | first `complementary` commit (`02817c0`) | **2027-02-08** |
| Failure-triggered parse-directed retrieval | 2026-02-09 | `PostToolUseFailure` path (`dc21343`) | 2027-02-09 |
| Blame-provenance bridging | 2026-02-11 | `docs/plans/file-classification-bridging.md` (design, predates code); `BridgeMode` code 2026-02-26 (`5686e1a`) | 2027-02-11 |
| Asymmetric query preprocessing | 2026-02-18 | `src/search/preprocess.rs` added (`164751e`) | 2027-02-18 |
| Self-supervised calibration from repo history | 2026-02-25 | `src/cli/calibrate.rs` added (`1753e04`); `docs/plans/adaptive-defaults.md` same day | 2027-02-25 |
| Feedback storage / rating loop | 2026-03-02 | `src/storage/feedback.rs` added (`aa1f6e5`) | 2027-03-02 |
| Intent-conditioned retrieval parameters | 2026-03-05 | `src/search/intent.rs` added (`ba5c035`) | 2027-03-05 |
| Session-ledger delta injection | 2026-03-05 | `ledger.jsonl` introduced (`1831e0e`) | 2027-03-05 |
| Query-overlap-weighted feedback propagation | 2026-03-22 | `file_feedback_scores` (`91fe8b1`) | 2027-03-22 |
| Cross-repo coupling via issue-ID trailers | 2026-06-27 | `src/index/cross_repo.rs` added (`0edae39`) | 2027-06-27 |

**Controlling statutory deadline for cluster B: 2027-02-08** (abstain gate and
complementary expansion). Note the combination claim (the closed-loop injection
controller) is only as young as its oldest disclosed element, so **treat
2027-02-08 as the deadline for the whole of Provisional B**.

## Practical deadline

Both provisionals should be **filed by 2026-08-31** — not because of the
statutory windows above, but because the inventor begins employment on
2026-09-01 under an agreement whose invention-assignment clause may reach
future filings. Filing before the start date, and listing both applications
and all seven repositories on the employer's prior-inventions exhibit,
establishes these as pre-existing inventions.

## Caveats

- Commit author dates can precede public push dates. If any push lagged its
  commit by a material interval, the true disclosure date is later (which only
  *extends* a deadline, never shortens it) — the dates above are therefore
  conservative in the safe direction.
- Older, related disclosures may exist outside these two repos (e.g. sibling
  repos describing a mechanism earlier). A pre-filing check of camayoc,
  yupana, shantytown, NeuralAmplifier and creel histories for earlier enabling
  descriptions of any claimed mechanism is recommended.
- Micro entity vs small entity: because the inventor has signed an employment
  agreement that may create an assignment obligation, **small entity**
  certification is the safe choice; undercertification risks unenforceability,
  overpayment carries no penalty.
