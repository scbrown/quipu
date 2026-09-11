# Design: The Signing Plane — governing the trust root like everything else

> **Implementation status (2026-09-11):** **v1 verdict signing and verifier
> registration are implemented (§2).** Native session/share attestation also
> ships with a protected binding registry and durable nonce replay protection
> (§2.1). **The signing-plane governance proposals in §5–§7 remain future work.**
> The implementation and regression tests below identify these separate scopes.
> This design originated in a session with Stiwi; the task-signing concept
> (§6) is human-originated (Stiwi, 2026-08-09).

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

Implementation anchors: [`src/signing.rs`](../../src/signing.rs) supplies the
Ed25519 primitives and `sign_roundtrips_and_rejects_tampering` regression;
[`src/governance/verdict_facts.rs`](../../src/governance/verdict_facts.rs)
emits signed verdict facts. `test_signed_verdict_end_to_end_root_of_trust` in
[`src/mcp/tests.rs`](../../src/mcp/tests.rs) proves that a registered, authorized
signer's verdict verifies as trusted and that tampering breaks the seal.

### 2.1. Session/share attestation — implemented, separate from verdict registration

The common verifier in
[`src/session_attestation.rs`](../../src/session_attestation.rs) handles
`quipu-write-v1` and `quipu-share-v1` as distinct canonical payload domains.
`both_domains_use_one_verifier_and_distinct_canonical_builders` and
`tamper_substitution_replay_and_domain_downgrade_are_rejected` cover that
boundary. Share import reaches it through
[`src/share_attestation.rs`](../../src/share_attestation.rs).

Session bindings and spent nonces live in protected SQLite tables
([`src/store/attestation.rs`](../../src/store/attestation.rs)), separate from
graph-writable `VerifierRegistration` facts. Nonce spending participates in
the mutation's savepoint: rollback returns the nonce, accepted mutations keep
it spent, and the spend survives reopening the store. These are separate
regressions in
[`src/store/attestation_tests.rs`](../../src/store/attestation_tests.rs):
`a_rolled_back_mutation_gives_the_nonce_back`,
`an_accepted_mutation_keeps_the_nonce_spent`, and
`a_spent_nonce_is_still_spent_after_a_reopen`.

Native imports distinguish **transport** (no envelope), **claimed** (a valid
self-carried identity without a local binding), and **attested** (verified
against an independently registered binding). Import never registers its own
producer: `import_does_not_register_the_binding_it_carries` and
`registering_out_of_band_reaches_attested` in
[`src/share_import_attestation_tests.rs`](../../src/share_import_attestation_tests.rs)
exercise that distinction through the real import path. The native CLI
provides `share --attest` and `attest register`; see the
[CLI sharing reference](../book/src/reference/cli-sharing.md). Browser imports
do not provide this attestation verifier.

This nonce replay protection prevents reusing an attestation. It does not
implement the historical, as-of trust-root verification proposed below, or
§6's task-scoped capability model.

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
