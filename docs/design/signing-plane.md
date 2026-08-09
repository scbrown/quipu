# Design: The Signing Plane — governing the trust root like everything else

> **Implementation status (2026-08-09):** ⬜ **Proposed, nothing built.**
> Distilled from a design session with Stiwi; the task-signing concept
> (§6) is human-originated (Stiwi, 2026-08-09). v1 signing as described
> in §2 is live (`src/signing.rs`, `src/governance/verdict_facts.rs`,
> `quipu_verdict_verify`); everything from §5 on is future work.

## 1. The question

Verdict signing today spans two systems: quipu signs and verifies, and
every governed writer (yupana first) re-implements the same scheme by
convention. Is signing worth bringing in-store the way policies were —
Σ as facts, the audit as a query? Answer: **the *governance* of signing,
yes; the keys themselves, no.** The boundary between those two is the
design.

## 2. What exists (v1)

- **Signing**: ed25519 (`ring`), canonical message
  `v1|predicate|target|outcome|evidenceHash|tier|verifier`, hex
  encodings. The evidence hash seals attribution — actor and principal
  chain — since Q-VERDICT-ATTRIB. No signing identity ⇒ **no verdict,
  never an unsigned one**.
- **Custody**: private keys are host files (`QUIPU_SIGNING_KEY`, 0600,
  generated on first use). Explicitly v1 — not an HSM or secret store.
- **Trust root, already in-store**: human-authored
  `aegis:VerifierRegistration` facts carry each verifier's name, public
  key, and the predicates it may attest. `quipu_verdict_verify` decides
  `trusted` = signature-valid ∧ registered ∧ authorized — all by query.
  Quipu never self-registers.
- **The mirror**: yupana's `src/verdict.rs` states "signing MIRRORS
  quipu's `signing.rs` exactly… Diverge from that scheme and the
  signature would be well-formed but never TRUSTED." Two codebases, one
  scheme, kept identical by a doc comment.

The "separate system" is therefore two distinct things: (a) key custody
outside the store, and (b) the scheme duplicated per writer with nothing
checking the copies agree. (a) is load-bearing (§4); (b) is debt (§5).

## 3. What replay actually covers today — measured honestly

The paper's RQ5 claim is precise and verified (CEN-M2,
`examples/census/phase6.rs`), and it is narrower than "everything
replays":

- **Satisfied verdicts re-derive fully** — claim-as-of over data-as-of
  (`query_temporal`) reproduces the decision. 50/50 in the seeded run,
  across the amendment boundary; all 50 would misreport under a
  latest-only Σ.
- **Denials verify rules-in-force only** — the staged delta was rolled
  back (GS2, deliberately), so replay confirms the policy and claim
  cited were in force at the instant, and does not re-derive the
  outcome. 6/6. The asymmetry is a finding, not a bug.
- **Not replayed at all**: authority-intersection outcomes (only their
  rules-in-force are checked, as denials), lattice-composition
  decisions (RQ4 probes run live, not as-of), and — the gap this doc
  exists to close — **signature verification**.
  `is_registered_verifier` and `registered_public_key`
  (`src/mcp/mod.rs`) query the registry with
  `TemporalContext::default()`: latest-only. There is no key history to
  ask an as-of question of.

Consequence: rotate a key and every historical verdict verifies against
the wrong key or none. Every *decision* in the store replays to the
extent its inputs were kept; the *trust root* does not replay at all.
That is D2's "one time axis, or none" alive inside the governance plane.

## 4. The boundary: what stays outside, on principle

Private keys and the act of signing stay outside the store — per
verifier, forever, not as v1 debt:

- If quipu held a writer's key (or exposed a signing service), a
  writer's signature would prove nothing: the store could mint
  attestations for its own writers — the self-vouching failure the
  registry design refuses ("quipu never self-registers").
- The `tier` on a verdict is honest only when the attesting party ran
  the analysis: a `tree-sitter` verdict must be signed by the process
  that parsed, not by the store that received.
- Custody *mechanism* (host file → secret store/HSM) is the real
  HARDEN-LATER item and remains external under every option below.

The store's job is to know **whose key was what, when, for which
scope** — never to hold the key.

## 5. Work items

### S0 — one signing crate, not two mirrors

Extract the v1 scheme (message format, hash, encodings, key I/O) into a
shared crate both quipu and yupana depend on. Ends the
"mirrors exactly" convention immediately; no trust moves anywhere.
Cheapest item, unblocks nothing but protects everything.

### S1 — bitemporal key registry (the prerequisite for the rest)

`aegis:publicKey` (and the registration's attest-scope) get
`valid_from`/`valid_to`, exactly as shapes did
([shape-versioning.md](shape-versioning.md)). Verification takes the
verdict's instant and answers against the key **registered then**:
as-of replay extended to the trust root — GS6 for signatures. Rotation
is a close-then-insert; revocation is a close; expiry is absence.
CEN-M2 grows a column: verdicts whose seal re-verifies as-of.

### S2 — the scheme as a versioned fact

`aegis:SigningScheme` facts carry the canonical message format, hash
suite, and signature algorithm, versioned bitemporally. Writers fetch
and self-test against the declared scheme; quipu refuses verdicts
citing a retired version. A `v2` rollout becomes a bitemporal amendment
instead of a synchronized multi-repo deploy.

### S3 — registry amendments through the gate

"Who signs the registry" (deferred in v1) gets the same answer policies
got: registration writes go through the write gate under a
meta-authority policy — only a human trust-root identity may amend
`VerifierRegistration`; N-of-M is a later tightening of the same
policy. "Quipu never self-registers" becomes an enforced claim in Σ
rather than a convention.

### S4 — depends on S1, S3.

## 6. Task signing (Stiwi, 2026-08-09) — attestation as a task-scoped capability

The concept: **an agent receives a task and holds no key beyond what
the task itself confers. It can attest only to entities related to that
task.**

Sketch, using the machinery above:

- A **task** is a first-class fact, minted by a principal whose
  authority covers the task's scope (target graphs, predicates,
  entities) and window.
- The **agent (or its harness) generates the keypair**; the store never
  sees the private half. The task minter registers the public key as a
  *scoped* `VerifierRegistration`: attests only within the task's
  scope, `valid_from`/`valid_to` = the task window.
- **Possession of the task key is the capability.** An attestation
  outside the scope fails the registration check — not a policy the
  agent might argue with, but an authority the registry never granted.
  No ambient identity exists to escalate.
- **Delegation only narrows** (GS3, extended to attestation): a task
  may mint subtasks whose registrations carry a subset of its scope,
  never more. Empty intersection refuses.
- **Expiry is absence**: the window closes, the registration's
  `valid_to` arrives, new attestations refuse — and the task's
  *historical* attestations still verify, because S1 answers as-of the
  attestation's instant. This is why S4 depends on S1: ephemeral keys
  are useless under latest-only verification, since every completed
  task's verdicts would go unverifiable the moment its key lapses.
- S3 supplies the discipline for who may mint tasks at all: task
  creation is a registry amendment, gated like any other.

Affinity worth noting: the parked WorldKernel framing
([paper.md](paper.md) §10) casts a knowledge-pack export as a
task-scoped admissible world. A task capsule then has two halves —
the pack bounds what the agent may *see*, the task key bounds what it
may *vouch for*. Same scope, read side and write side.

## 7. Paper angle (future work, not this revision)

S1+S3 complete the D4 story in a way none of the SARC line claims: the
trust root itself is bitemporal, governed data — "was this seal valid,
under the key registered at that instant, granted by an authorized
amendment?" is a query. §3's honesty about what replays today is the
baseline that result would be measured against.

## 8. Scope boundaries

- No HSM/secret-store integration here; custody hardening is orthogonal
  and stays external (§4).
- S2 versions the scheme, it does not design `v2` of the scheme itself.
- Task signing binds attestation scope; it is not sandboxing — nothing
  here prevents an agent from *doing* out-of-scope work, only from
  getting it attested and admitted.
