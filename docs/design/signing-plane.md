# Design: The Signing Plane — governing the trust root like everything else

> **Implementation status (2026-09-30):** **S1, the bitemporal verifier
> registry, is implemented** (aegis-kzt0ql.9.1; see §5 S1).
> **v1 verdict signing and verifier
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

`quipu-write-v2` (aegis-72cpbx) is v1 plus an **audience**: the receiving
store's `store_id`, which `GET /stats` reports. A v1 message names no server,
so a write one quipu accepted could be relayed to another quipu that trusts
the same key, and it verified there because nonces are per store. Under v2 the
server signs its OWN id into the message and compares the envelope's
`audience` claim against it first. A relay is refused as `invalid` (audience
mismatch). An envelope whose claim was rewritten to the relay target passes
that comparison and fails the signature (`badsig`). A v1 envelope that carries
an `audience` is `invalid`, because v1 never signs that field. The pinning
regressions are in
[`tests/signed_writes_server.rs`](../../tests/signed_writes_server.rs)
(`a_v2_write_signed_for_one_store_is_refused_by_another_as_invalid`,
`swapping_the_envelope_audience_to_the_relay_target_fails_the_signature`),
and the published vector is `tests/vectors/write-attestation-v2.json`.
v1 stays accepted, and its v1 relay is pinned by
`baseline_a_v1_write_accepted_by_one_store_is_accepted_by_another`; the v1
sunset is a separate decision. **Limit:** `store_id` is lineage, not identity.
A file-level copy of a store (a backup restore or a fork) keeps the id, so v2
does not separate a store from its own copy. Those two already trust the same
bindings.

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

### 2.2. Hardware verdict schemes — implemented, off by default

[`src/verdict_schemes`](../../src/verdict_schemes/mod.rs) adds three schemes
beside raw ed25519 so a human can attest a verdict with a device whose private
key never leaves it: `webauthn-es256` and `webauthn-eddsa` (passkeys and
security keys; the assertion signs `authenticatorData || SHA-256(clientDataJSON)`)
and `sshsig-sk-ed25519` (`ssh-keygen -Y sign -n quipu-verdict` with a FIDO
`sk-ssh-ed25519@openssh.com` key). All sign the same v1 canonical message.

A registration declares its scheme with `aegis:signatureScheme` (absent means
`ed25519`, so existing registrations are unchanged), and a verdict verifies
only against a registration declaring the scheme it names. Quipu derives the
WebAuthn challenge from the verdict message and never takes one from the
caller; it checks the origin, the RP ID hash, the SSHSIG namespace, and a
non-regressing authenticator counter (WebAuthn Level 3 §7.2: once a nonzero
counter is recorded, a counter that does not exceed it, including 0, is
refused). "Hardware-backed" means user presence is always required; user
verification (biometric or PIN) is required for WebAuthn and only reported for
SSHSIG, where it depends on the key being created with `verify-required`. The counter to record is returned; verification itself stays
read-only.

The schemes are **off by default** (`[quipu.governance]
hardware_verdict_schemes`). Until S3 (§5) restricts `VerifierRegistration`
amendments to an enrolled human key, a registration is still graph-writable,
so enabling them is an operator decision. While off, both a verdict naming a
hardware scheme and a write declaring one are refused. The whole-decision
canonicalization and single-use nonce of the hardware-verdict design are
separate work; this scheme layer signs the v1 message.

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
  ask an as-of question of. *(Historical: closed by S1, 2026-09-30.)*

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

> **Implemented** in [`src/governance/verifier_registry.rs`](../../src/governance/verifier_registry.rs).
> Registrations are ordinary graph facts, so no schema was needed: rotation
> retracts the old `aegis:publicKey` and asserts the new one, revocation
> retracts it, and expiry is a `valid_to` set in the future. Every verifier
> (verdict verify, the escalation router and its precedent search, the
> transition gate) asks the registry through one function, `registered_keys`,
> and the key must be a fact of the same registration that grants the scope.
>
> **The instant is the store's, not the signer's.** A `Witness` is the
> transaction that first recorded the signature, plus that transaction's
> timestamp. A registration counts only if it was asserted by that tx and not
> yet closed by it. Tx ids are monotonic, so a revoked key cannot back-date a
> signature into its old window. The registration's valid interval must also
> cover the timestamp, which lets a compromise revocation reach back to an
> earlier instant. Signatures being written in the current transaction (the
> write gates) verify at now. `quipu_verdict_verify` takes `verdict` (the
> stored verdict's IRI) to verify as-of its recording, and reports
> `as_of.basis`. Tests: `verifier_registry_tests.rs`, `s1_*` in
> `router_tests.rs`, and `test_verdict_verify_answers_as_of_the_recorded_signature`.

`aegis:publicKey` (and the registration's attest-scope) get
`valid_from`/`valid_to`, exactly as shapes did
([shape-versioning.md](shape-versioning.md)). Verification takes the
verdict's instant and answers against the key **registered then**:
as-of replay extended to the trust root — GS6 for signatures. Rotation
is a close-then-insert; revocation is a close; expiry is absence.
CEN-M2 grows a column: verdicts whose seal re-verifies as-of.

### Sealed decisions (aegis-kzt0ql.9.3) — sign the whole decision

> **Implemented** in [`src/governance/decision_seal.rs`](../../src/governance/decision_seal.rs).
> `decision-v1` signs `evidenceHash|outcome|by` only, so a decision's
> question, options, scope and expiry could be rewritten after signing and
> the ruling would still stand (pinned by
> `v1_decisions_do_not_seal_their_content_use_decision_seal`).
> `quipu-decision-v2` seals the content:
>
> 1. `present` computes `sha256` over the RDFC-1.0 canonical form of the
>    decision's concise bounded description (its facts in ROOT, plus the
>    blank nodes it reaches; only attestation fields are excluded), mints a
>    nonce, and records an `aegis:DecisionPresentation`.
> 2. The approver signs
>    `quipu-decision-v2|quipu-verdict|decision|digest|outcome|nonce|expiresAt`.
> 3. `attest` RECOMPUTES the digest from the stored decision and never takes
>    a supplied one. It checks the digest equals the frozen one, checks the
>    outcome is a declared `aegis:option`, and verifies against a key
>    registered now for `aegis:decisionPolicy`. It then spends the nonce and
>    records the `aegis:DecisionVerdict` in ONE savepoint (`decision_nonces`,
>    never pruned).
> 4. `verify_recorded` recomputes the digest from the decision as it stands
>    now (so an edit after signing invalidates the verdict) and verifies as
>    of the recorded signature (S1). It also requires that `attest` admitted
>    the verdict: the verdict is `decision_verdict_<nonce>`, the nonce's
>    spend row names this decision and this verdict, the spending
>    transaction wrote every load-bearing verdict fact (`forPresentation`,
>    `outcome`, `verifier`, `sealSignature`), and it was recorded before
>    `presentationExpiresAt`. Verdicts are graph-writable until S3, so
>    without this a hand-written verdict carrying a captured signature (one
>    `attest` refused as expired, or never received) would verify, and so
>    would a second verdict on a spent nonce (wu-rev-345 F1).
>
> **Limits of the seal (wu-rev-345 F2, F4).** The digest covers the
> decision's own ROOT facts and their blank-node closure, nothing more.
> An object that is an IRI (an `aegis:authorizedAction <x>`, a warrant-scope
> entity) is sealed BY REFERENCE: a later change to that entity's own facts
> is not detected. Facts about the decision in NAMED graphs are not sealed,
> because `entity_facts` reads ROOT; a reader that resolves decision
> content across graphs can read something other than what was sealed.
> .9.6 should consider sealing the closure of the scope and action
> predicates too. The digest also depends on the lexical form quipu emits
> for each literal, so a future literal re-encoding (e.g. numeric
> identity, #320) changes every sealed digest and turns open presentations
> into `ContentChanged`. That fails closed.
>
> **Wiring rule (F3).** `present(now)` and `attest(now)` have no production
> caller yet. When they are exposed over MCP or REST, `now` MUST be the
> server clock and never input, or the expiry becomes caller-controlled
> (the same rule as S1's caller-supplied instant).
>
> The seal uses its own predicates where a shared one carries an
> `rdfs:domain` that inference would apply (`decisionPolicy`,
> `presentationExpiresAt`, `sealSignature`). Presentation writes are
> graph-writable until S3 gates them. That cannot forge a verdict (the
> digest is recomputed and the key is checked), but an agent could choose
> a presentation's nonce or expiry, and the approver sees both in the
> challenge.

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
