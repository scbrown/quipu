# Filing record — Provisional A (this repo's cluster)

**Status:** ✅ **FILED 2026-08-17.** Supersedes the "remaining gate" language in
[`filing-cover-sheet-data.md`](filing-cover-sheet-data.md), which was written
before filing.

> This repo is public. Personal fields (full legal name, residence, mailing
> address) are deliberately **not** recorded here, exactly as the cover-sheet
> doc established. They live in the inventor's private records.

## A — this repository

| Field | Value |
|---|---|
| **Application number** | **`64/135,410`** |
| Confirmation number | `7462` |
| Attorney docket | `SCB-001-PRV` |
| Title | Governed Knowledge Store with Rollback-Surviving Signed Verdicts, Composable Graph Labels, and Bitemporally Versioned Validation |
| Specification filed | [`provisional-A-governance.pdf`](provisional-A-governance.pdf), 35 pp |
| Filed | 2026-08-17, 4:11:38 PM ET |
| Type | Provisional under 35 U.S.C. 111(b), Utility |
| Entity status | Small (not micro — the inventor's prior-year gross income exceeds the micro ceiling) |
| Inventor | Sole |
| Assignee | **None. Unassigned.** |
| **Expires** | **2027-08-17. Not extendable.** |

Provisionals are never examined and never publish. This one grants nothing on
its own; it fixes a priority date of **2026-08-17** for whatever the
specification supports.

## The sibling filings

All four were filed the same day, all sole-inventor, small entity, unassigned,
$130 each.

| | Repo | Application | Docket |
|---|---|---|---|
| A | **quipu** (this one) | `64/135,410` | SCB-001-PRV |
| B | bobbin | `64/135,383` | SCB-002-PRV |
| C | NeuralAmplifier | `64/135,421` | SCB-003-PRV |
| D | camayoc | `64/135,436` | SCB-004-PRV |

## 🔴 Disposition: A is planned to lapse

**Only B is being converted to a nonprovisional.** The goal is a granted patent
as a durable credential rather than licensing revenue, and converting four
costs four times as much for no additional credential value.

**A is the designated backup.** If the prior-art search on B comes back badly,
A is the fallback, and the same plan applies to it. A's strongest candidate
claim is verdict staging with post-savepoint flush — recording a denial that
survives the rollback of the very write it judged — which reads well as a
concrete improvement to database machinery rather than an abstract idea. The
headwind is that "log survives rollback" already exists in the wild (Oracle
autonomous transactions), so any claim would rely on the narrower combination:
evidence-hash-bound signing, the deterministic subject identifier making
re-recording idempotent, and the re-entrancy carve-out that stops a policy
denying the record of its own denial.

**If nothing changes, A lapses on 2027-08-17** and its mechanisms remain
published prior art that no one else can patent.

## Grace periods: satisfied

[`disclosure-timeline.md`](disclosure-timeline.md) computes per-mechanism
deadlines from first public disclosure. Cluster A's controlling date is
**2027-07-19**. The 2026-08-17 filing lands inside every window, so no
disclosure in this repo is prior art against this application.

Non-US rights were foreclosed by those same public disclosures — absolute
novelty, no grace period — and are not part of the plan.

## For agents working in this repo

- Do not describe A as "pending patent protection". A provisional confers no
  enforceable rights.
- "Patent pending" is accurate only while an application is alive.
- Adding new mechanisms to this repo does **not** extend A. Anything disclosed
  after 2026-08-17 that is not supported by the filed specification has its own
  fresh 12-month clock and is not covered.
