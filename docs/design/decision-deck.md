# Decision Deck native backend

Status: native implementation, disabled by default.

- The HTTP routes `/deck/present`, `/deck/attest` and `/deck/receipt` exist
  behind `[quipu.server] deck_http`.
- The operator CLI `quipu deck` configures, enrolls, enables, revokes and
  reports over the store file (see "Operator CLI" below).
- There is no MCP enrollment, no browser enrollment ceremony and no
  production caller.
- Temporary-store tests use software authenticators and establish no human
  identity.

## Trust and scope

`store::decision_deck` authenticates one explicit action using an independently
enrolled WebAuthn credential. It reuses the ES256/EdDSA assertion verifier: exact
origin, RP hash, signature, user presence and user verification are required.
It does not attest a URL path, document bytes, physical device custody or an
unobserved human identity. Use a dedicated HTTPS origin; a shared origin does
not isolate sibling pages. Browser trust and enrollment require separate acceptance.

Native database-file access is operator authority. `deck_configure` installs
one immutable application scope: exact HTTPS origin/RP host, actor and sole
request curator. It starts disabled; `deck_set_enabled` is an explicit native
operator action. No graph write, import, browser field or session binding can
configure the application, enroll a credential or enable admission. Future
transport adapters must independently authorize operator methods: accepting a
self-asserted curator name or a public key from the submitting page is insufficient.

The protected SQLite tables retain public credential material, permanent
revocation and counters, curated requests, challenges, receipts and transaction
links. They contain no private keys. Store-file copies preserve lineage and
authority state; they are trusted backups, not independent hosts or enrollments.
Ordinary graph exports do not carry protected enrollment or nonce authority.

## Request and challenge contract

`deck_put_request` accepts a curated `DecisionRequest`: absolute IRI, strictly
increasing authoritative revision, policy, target, exact proposition, expiry,
curator and a nonempty subset of `approve`, `reject`, `changes`. Free prose from
another application is not converted into a request. A changed proposition is
a new authoritative revision, invalidating outstanding challenges.

`deck_present` takes the exact request revision, enrolled credential and declared
choice. The server supplies its clock, store lineage and a random 256-bit nonce.
It caps expiry at five minutes and the request deadline. The serialized typed
`Challenge` contains the full request, choice, origin, RP and actor, under the
`quipu-decision-deck-v1` domain. The browser passes
`Challenge::webauthn_challenge()` to `navigator.credentials.get()` with the exact
credential and `userVerification: "required"`. This uses the existing verifier's
domain-separated challenge derivation over the complete action message; it is
not a new generic verdict or session-attestation format.

`deck_attest` accepts only a nonce and raw assertion bytes. It retrieves the
authoritative action itself, rejects replay, stale versions, changed scope,
expired requests/challenges and revoked/expired credentials, then verifies the
assertion against the enrolled public key. Typed client-data parsing rejects
ambiguous duplicate fields and malformed `crossOrigin`. Existing counter policy
is retained: both counters may be zero; otherwise a recorded nonzero counter
must advance. The random challenge ledger remains the replay authority.

## Atomic settlement and recovery

The native writer requires an independent SQLite transaction and obtains writer
ownership before reading request/challenge state. Challenge consumption, unique
`(request, revision)` admission, counter advance, receipt, transaction linkage and
the projected `Decision` facts share one savepoint. The normal staging and
governance guards still run. Observers run only after settlement commits. A late
settlement failure rolls back both protected state and projected facts.

Only the protected native writer may mutate `urn:quipu:decision-deck`. Generic
graph writes are refused even when their actor/source strings imitate the
curator. Queries must explicitly select that graph. The projection uses the
existing governed `DecisionRequest` and `Decision` classes and required fields.
It does not convert a decision into permission to execute its target.

A second nonce cannot decide the same request revision again. After an
indeterminate response, call the read-only `deck_receipt(nonce)`; do not mint a
new action automatically. It returns the exact immutable receipt and committed
transaction. A repeat submission remains refused.

`deck_verify_receipt(nonce)` re-verifies a protected, admitted receipt using its
retained public key and server witness. Later revocation blocks new admissions
without erasing past decisions. This historical result is not current execution
authority. Raw assertions, client bytes, complete action, key, scheme and witness
are retained for independent verification.

`audit_query()` returns the bounded stored-query definition
`decision_deck_audit_v1` (explicit graph, limit 200). An authorized operator may
install it with `Store::query_load`. It lists admitted receipt projections;
query text and a returned row alone are not signature verification. Re-verify
the protected receipt before trusting a result. Receipt lookup works while the
application is disabled or the credential has been revoked.

## Operator CLI

`quipu deck` is the only caller of the operator methods outside tests. It
acts on the store file, so file access remains the authority, and it has no
HTTP route:

```text
quipu deck configure --origin https://HOST --rp-id HOST --actor WHO --author CURATOR
quipu deck enroll --credential-id B64URL --cose-key B64URL --scheme webauthn-es256|webauthn-eddsa --expires-at EPOCH
quipu deck enable | disable
quipu deck revoke <CREDENTIAL-ID>
quipu deck status
```

`enroll` and `status` print the credential's fingerprint: the first 16 hex
digits of SHA-256 over the exact COSE key bytes, in groups of four (for
example `9bb1-7e67-b268-ca8b`). An enrollment page shows the same value,
computed from the bytes it hands over. Before enabling, the person enrolling
confirms that the two match. A match names the key; it does not prove who
holds it. A mismatched enrollment is revoked, never edited.

## Remaining integration gates

Before any production caller is added: accept the dedicated trusted TLS origin,
exact-origin browser permissions and connector; review the narrow transport,
bounded challenge issuance, enrollment authentication and refusal alerting; and
observe one real owner enrollment and decision with forged/replay negatives and
the audit query. No bearer or private signing key belongs in browser code.
Origin/presence claims need actual browser acceptance, beyond the software tests.

Rollback disables admission through the native operator switch and removes a
future transport caller. Retain receipts, revocation and consumed challenges;
do not roll a database back to restore unspent approval state. Source rollback
does not erase the evidence ledger.

## Verification

Run `just test --no-default-features --lib store::decision_deck` for temporary-store
controls. These cover both algorithms, protected authorship, forgery, wrong
origin/RP/presence, stale versions, expiry, revocation, default-off behavior,
semantic duplicates, late transaction failure and durable response recovery.
The existing WebAuthn suite remains an independent positive and negative control.
