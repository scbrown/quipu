# Prior-Art Search Notes — Provisional A (Governance Cluster)

**Status:** preliminary keyword sweep of Google Patents, 2026-08-15. This is
an agent-run scoping search, NOT a professional patentability search and not
legal advice. Its purpose is (a) to give the attorney a head start and (b) to
sharpen claim drafting for the nonprovisional. A provisional application
requires no prior-art citations; nothing here creates a duty of disclosure
until a nonprovisional is filed, at which point known material references
must go on an IDS — keep this file current with anything found.

## Method

Keyword searches over `patents.google.com` (plus incidental non-patent
literature), one sweep per mechanism cluster. Keyword search is weak
against patents that use different vocabulary for the same mechanism; a
professional search with classification codes (e.g. G06F 16/xx, G06F 21/64,
H04L 9/32) should be run before conversion.

## Findings by mechanism

### 1. Rollback-surviving verdicts (§ 2)

- [US8332349B1](https://patents.google.com/patent/US8332349B1/en) —
  asynchronous ACID event-driven audit-trail processing. Audit history in
  locked-down tables; discusses rollback semantics. Closest found to the
  audit-across-rollback problem; does not show staging decisions in memory
  during the judged savepoint and flushing after resolution into the same
  governed bitemporal store, nor signed content-idempotent verdicts.
- [US20100115284A1](https://patents.google.com/patent/US20100115284) —
  tamper-evident audit logs via overlapping chains of signed record
  subsets. Signs the *log*; separate-lifecycle log, not decision records
  co-resident with governed data surviving the judged write's rollback.
- [US7814075B2](https://patents.google.com/patent/US7814075) — dynamic
  auditing. General audit-policy machinery; no rollback-survival ordering.
- The spec already distinguishes autonomous transactions (Background ¶1,
  § 2.5); that distinction covers the closest non-patent mechanism.

### 2. Signed policy-decision attestations (§ 2.3)

- [US9411962B2](https://patents.google.com/patent/US9411962) — attestation
  in policy-based decision making (MDM). Attestation as *input* to policy
  decisions, not signed records *of* decisions stored with the data.
- [US9716594B2](https://patents.google.com/patent/US9716594B2/en) — signed
  data-sanitization attestation. Signed attestation of an operation, but
  standalone artifact, no evidence-hash binding of attribution, no
  in-store bitemporal residence.
- [US11947523B2](https://patents.google.com/patent/US11947523B2/en) —
  multi-party signature policies per key in a KV database. Signatures
  authorize writes; does not attest policy outcomes or survive rollback.

### 3. Request-bound escalation, expiry-as-denial (§ 3)

- [US8725548B2](https://patents.google.com/patent/US8725548) /
  [US7131071B2](https://patents.google.com/patent/US7131071B2/en) — generic
  dynamic approval workflows. No deterministic request identity from
  (policy, target) digest, no expiry-as-standing-denial, no
  rejection-outranks-approval rule.
- [US20050022009A1](https://patents.google.com/patent/US20050022009) —
  replay-attack prevention (Bloom filters). Different problem shape; the
  approve-then-change defense via digest binding did not surface in patent
  results — this absence is encouraging for § 3 but keyword-weak.

### 4. Label lattice with coverage identity (§ 4)

- [US20060206485A1](https://patents.google.com/patent/US20060206485A1/en),
  [US9514328B2](https://patents.google.com/patent/US9514328B2/en),
  [US20050289342A1](https://patents.google.com/patent/US20050289342) —
  MLS/row/column security labels with dominance lattices. Classic
  Bell-LaPadula-style single-axis dominance; none shows per-axis
  composition direction under a never-widens invariant, coverage as a
  distinguished non-declarable fold identity, chain-scoped trust with
  cross-chain refusal, or drift-refusing cached labels. The MLS family is
  the art the examiner will reach for on aspect 13 — claim language should
  keep the coverage-pair and cross-chain-error limitations prominent.

### 5. Bitemporal rule versioning + fidelity/drift replay (§ 5)

- Patent keyword results were thin; the strong art is academic (temporal
  schema versioning literature, e.g. multitemporal relational schema
  versioning). No patent found combining as-of *validation* (rules in
  force at T applied to data) with the fidelity/drift replay split.
  US12524383 (bitemporal object management) is general bitemporal storage.

### 6. Monotone two-pass contextual validation (§ 6)

- No patent found. The live art is non-patent literature: SHACL validation
  under graph updates ([arXiv:2508.00137](https://arxiv.org/pdf/2508.00137)),
  SHACL-DS dataset validation ([arXiv:2505.09198](https://arxiv.org/pdf/2505.09198)),
  SHACL with ontologies ([arXiv:2507.12286](https://arxiv.org/pdf/2507.12286)).
  None describes the ceiling/subset-repair construction; they should be
  read before drafting nonprovisional claims and cited on the IDS if
  material.

### 7. Adjacent agentic-governance filings (landscape, all clusters)

Recent applications show the space is filling fast — filing sooner
protects more:

- [US20260017525A1](https://patents.google.com/patent/US20260017525A1/en) —
  validating autonomous AI agents (prevent unauthorized actions, enforce
  compliance).
- [US12437058B1](https://patents.google.com/patent/US12437058B1/en) — LLM
  security threat mitigation with action validation.
- [US12412138B1](https://patents.google.com/patent/US12412138B1/en) —
  agentic orchestration with compliance features.
- [US20250252293A1](https://patents.google.com/patent/US20250252293A1/en) —
  LLM-driven agent orchestration.

These sit at the agent/orchestration layer, not the store layer; Provisional
A's positioning (in-store mechanisms, writer-agnostic) is the right
distinction and is already stated in the Background.

## Take-aways for conversion

1. Nothing found anticipates the five core mechanisms as claimed; the
   closest families are (1) audit-log tamper evidence and (4) MLS label
   dominance, both distinguished by limitations already in the aspects.
2. Run a professional search (classification-code based) before the
   nonprovisional; budget for it in the attorney engagement.
3. The agentic-governance space is accumulating filings dated 2025–2026;
   the practical deadline in `disclosure-timeline.md` (file by 2026-08-31)
   is reinforced, not relaxed, by this landscape.
