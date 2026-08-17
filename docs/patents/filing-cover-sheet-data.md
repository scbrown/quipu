# Filing-Day Cover Sheet Data — Provisionals A through D

**Status:** prepared 2026-08-14 for filing via USPTO Patent Center by
2026-08-31. ✅ **All four specifications are drafted with filing-ready PDFs as
of 2026-08-17** — A `quipu/docs/patents/provisional-A-governance.pdf` (35pp),
B `bobbin/docs/patents/provisional-B-retrieval.pdf` (37pp),
C `NeuralAmplifier/docs/patents/provisional-C-delegation.pdf` (~47pp),
D `camayoc/docs/patents/provisional-D-grounding.pdf` (~35pp). **The remaining
gate is the verified USPTO.gov account**, which has lead time and blocks all
four.

⚠️ `bobbin`'s remote is named `github`, not `origin`, and a stale `origin/main`
ref from 2026-08-05 is still present locally. Read `github/main` there. Personal fields are deliberately left as `[FILL AT FILING]`
placeholders — complete them in Patent Center, not in this public file.
Fee amounts should be re-verified against the
[USPTO fee schedule](https://www.uspto.gov/learning-and-resources/fees-and-payment/uspto-fee-schedule)
on filing day.

## Common to both applications

| Field | Value |
|---|---|
| Application type | Provisional |
| Subject matter | Utility |
| Inventor (sole) | `[FILL AT FILING — full legal name confirmed with the inventor 2026-08-17; the earlier "inferred from the GitHub handle" caveat is resolved. Deliberately not written here: this repo is public and the middle name is not.]` |
| Inventor residence | `[FILL AT FILING — city, state, country]` |
| Inventor mailing address | `[FILL AT FILING]` |
| Correspondence email | `scbrown3@gmail.com` |
| Correspondence address | `[FILL AT FILING — same as mailing address is fine]` |
| Entity status | **Small entity** (deliberate choice: a signed employment agreement may create an assignment obligation, which would defeat micro-entity certification; overpaying small-entity is always safe, undercertifying is not) |
| Provisional filing fee | **$130 small entity** ✅ verified against the fee schedule revised 2026-08-14 (undiscounted $325, micro $65) |
| Application size fee | **None owed.** 37 CFR 1.16(s) only bites above **100 sheets**; the four specs are 35, 37, 47 and 35 pages |
| Total for all four | **$520** |
| Government interest | None |
| Joint research agreement | None `[CONFIRM]` |
| Signature format | S-signature: `/Your Legal Name/` typed between forward slashes |
| Claims / oath / IDS | Not required for a provisional — file the specification only |

## Application 1 — Provisional A

| Field | Value |
|---|---|
| Title | Governed Knowledge Store with Rollback-Surviving Signed Verdicts, Composable Graph Labels, and Bitemporally Versioned Validation |
| Suggested docket number | SCB-001-PRV |
| Specification source | `quipu` repo, `docs/patents/provisional-governance-cluster.md` (~12,600 words, 8 figures) |
| Upload as | Single specification PDF with figures rendered inline |
| Statutory deadline protected | 2027-07-19 (earliest cluster mechanism; escalation-router row conservatively 2027-07-21) |

## Application 2 — Provisional B

| Field | Value |
|---|---|
| Title | Event-Driven Context Injection for Language-Model Coding Agents Using Version-Control Provenance, Self-Supervised Calibration, and Session-Scoped Delta Injection |
| Suggested docket number | SCB-002-PRV |
| Specification source | `bobbin` repo, `docs/patents/provisional-retrieval-cluster.md` (~11,100 words, 7 figures) |
| Upload as | Single specification PDF with figures rendered inline |
| Statutory deadline protected | 2027-01-02 for combination/coupling-expansion claims; individual mechanisms per the disclosure timeline |

## Application 3 — Provisional C

| Field | Value |
|---|---|
| Title | Deadline-Fenced Delegation of Decision Surfaces from a Synchronous Engine to an Asynchronous External Decision-Maker with Fallback-Gated Tiering |
| Suggested docket number | SCB-003-PRV |
| Specification source | `NeuralAmplifier` repo, `docs/patents/provisional-decision-delegation.md` (~15,000 words, 8 figures) |
| Upload as | Single specification PDF with figures rendered inline |
| Statutory deadline protected | 2027-07-26 (the contract disclosure; combination claims inherit it) |

## Application 4 — Provisional D

| Field | Value |
|---|---|
| Title | Grounding-Integrity Machinery for Machine-Written Knowledge: Provenance-Refusing Ingress, Quarantined Inference with Governed Promotion, Falsifier-Gated Verification, Tier-Honest Fact Serving, and Typed Non-Answers |
| Suggested docket number | SCB-004-PRV |
| Specification source | `camayoc` repo, `docs/patents/provisional-grounding-cluster.md` (~9 figures) |
| Upload as | Single specification PDF with figures rendered inline |
| Statutory deadline protected | 2027-07-18 (closed tier vocabulary, first disclosed in yupana while named hank) |

## Step 0 — the USPTO.gov account (the only remaining gate)

**Identity verification has been mandatory for every Patent Center user since
2025-09-11.** There is no filing without it. Two routes exist; only one is fast.

| Step | What | Time |
|---|---|---|
| 1 | Create the account at <https://account.uspto.gov/profile/create-account> | minutes |
| 2 | Verify identity through **ID.me** | **~30 min**, or under 15 on the self-service path |
| 3 | Log into [Patent Center](https://patentcenter.uspto.gov), self-enroll, role **Independent Inventor** | part of the same session |

🔴 **The name on the USPTO.gov account must be *exactly identical* to the name on
the government photo ID**, because ID.me builds the match from that ID. This is
the single most likely point of failure, and it is why the inferred-name caveat
above mattered. Use the full legal name as printed on the licence, not a
shortened or published form. **Do not create the account as "Steve Brown."**

**ID.me self-service needs:** the photo ID, a Social Security number, access to
credit-profile header data, and a selfie for biometric match *(ID.me states the
selfie is deleted 24 hours after account creation)*. The alternative is a live
video chat with an agent, where the wait depends on volume.

✅ **An existing verified ID.me account carries over.** ID.me verification is
reusable across federal and state agencies, so if one already exists from an
unrelated filing, step 2 may already be satisfied and this whole gate collapses
to roughly half an hour rather than the "days ahead" this document originally
assumed.

**Customer number:** not needed in advance. Self-enrollment completes without
one, and Patent Center creates a number afterwards. An existing customer number
can instead be self-assigned during enrollment.

**Slow fallback, avoid:** mailing a paper Patent Electronic System Verification
form. Only worth considering if ID.me fails outright, and it will not fit inside
the 2026-08-31 target.

## Filing order

**File B first.** All four are independent and each earns its own filing date, so
order is normally irrelevant. It matters only if a session goes wrong partway:
B protects **2027-01-02**, the earliest statutory deadline of the four, while A,
C and D all sit in July 2027. Front-load the one with the least runway.

Staged and verified 2026-08-17 in `~/Downloads/provisionals-2026-08-17/`.

## Filing-day sequence (per application)

1. Sign in to [Patent Center](https://patentcenter.uspto.gov) (verified
   USPTO.gov account required — start identity verification days ahead).
2. New submission → Utility — Provisional.
3. Upload the specification PDF (the markdown must be converted with all
   mermaid figures rendered — a raw markdown upload will not preserve them).
4. Enter the cover data above; Patent Center generates the provisional cover
   sheet (equivalent of form PTO/SB/16) from it.
5. Certify **small entity**; pay the fee.
6. Save the filing receipt and the application number (63/xxx,xxx).
7. After **all four** filings: record every application number, title, and
   filing date on the new employer's prior-inventions exhibit alongside the
   seven repositories (quipu, bobbin, yupana, shantytown, camayoc,
   NeuralAmplifier, creel).

## The 12-month decision (calendar these)

A provisional expires 12 months after filing. To keep either priority date,
file the corresponding nonprovisional (with formal claims — attorney or
law-school-clinic stage) before:

- Application A: filing date + 12 months
- Application B: filing date + 12 months
- Application C: filing date + 12 months
- Application D: filing date + 12 months

All four conversion decisions will fall during the new employment — the
prior-inventions exhibit listing is what keeps them clean. Raise the
inventorship analysis (human contribution per claimed mechanism, and the
§102(b)(1)(A) inventor-derivation chain behind agent-authored public
commits) with the attorney before converting.
