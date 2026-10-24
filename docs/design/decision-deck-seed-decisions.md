# Decision Deck: signed decisions on work items

Status: design, not implemented. It extends the Decision Deck
(`decision-deck.md`); everything there still holds unless this document says
otherwise. Nothing is enabled by writing it.

## Goal

The operator answers an open question on a work item from a browser card, by
choosing one of the presented options, choosing **Later**, or leaving a
**comment**. Each answer is signed on the operator's enrolled device and
verified by Quipu. It is recorded atomically, and it is readable as:

- the decision itself;
- a visible comment on the item;
- the item's disposition: answered, deferred to Later, or still waiting.

A stale card, a closed item, a replay or a forged assertion records nothing.

## What the Deck already provides

Read from `src/store/decision_deck/*` and `src/server/deck.rs`:

- **Curation:** a curated `Request` (IRI, strictly increasing revision,
  policy, target, summary, expiry, choices, author) is put by the sole curator
  through operator access.
- **Challenge:** `deck_present` mints a 256-bit server nonce and a typed
  `Challenge` binding the request, choice, origin, RP ID, actor and store. The
  challenge expires within 5 minutes.
- **Attestation:** `deck_attest` verifies the WebAuthn assertion against the
  enrolled credential. In one savepoint it consumes the challenge, inserts the
  receipt (with `UNIQUE(request, revision)`), advances the counter, and writes
  `Decision` facts into `urn:quipu:decision-deck`. Observers run after commit.
- **Recovery:** `deck_receipt(nonce)` recovers a lost response, and a
  re-submission is refused. `deck_verify_receipt` re-verifies history.
- **Switches:** both the server's `deck_http` and the store's operator
  `deck_config.enabled` must be on.

## Gaps for item decisions

1. Choices are limited to `approve | reject | changes`, at most 3. Work-item
   questions carry their own option labels.
2. There is no Later and no comment.
3. Nothing ties a request to the item's *live* state. If the item changes
   after curation, the old request revision still verifies.
4. There is no curator transport and no enrollment transport. `deck_put_request`
   and `deck_enroll` are native-only.

## Design

### 1. Labelled choices and actions (wire v2)

`Request` gains two optional fields:

- `labels`: 1 to 8 curated option labels. Each is 1 to 200 bytes of plain
  text, with no control characters, unique, and in display order. The curator
  takes them from the item's own options, or its title alternatives, or falls
  back to `Yes`/`No`.
- `actions`: the subset of `choose`, `later` and `comment` that this request
  allows.

A v2 request carries `labels` and an empty `choices`. A v1 request carries
`choices`, with no `labels` and no `actions`. A request cannot mix the two.

`Challenge` gains two optional fields:

- `action`: `choose`, `later` or `comment`.
- `comment`: up to 4,000 bytes of UTF-8 plain text. It is required for
  `comment`, and optional for `choose` and `later`.

For `choose`, `choice` must equal one of the request's `labels` exactly. For
`later` and `comment`, `choice` is empty.

**Wire compatibility.** A challenge with any v2 field uses domain
`quipu-decision-deck-v2`. Every new field is skipped when absent, so a v1
challenge serializes to the same bytes as today, and the frozen v1 golden
vector stays unchanged. A new frozen v2 vector is added. The comment is
inside the signed message, so the signature covers the exact text that is
recorded.

**Times.** Every time in v2 is the same as in v1:

- an `i64` of Unix epoch **seconds**;
- read from the server clock and never from the browser.

This covers `Request.expires_at`, `Challenge.expires_at` and
`Receipt.recorded_at`.

**Golden vector.** The v2 vector is committed with the implementation, in
`golden_tests.rs` next to v1, and is then frozen. It pins the exact serialized
challenge bytes (compact JSON in field order), the derived WebAuthn challenge,
and a serialized receipt. Clients must reproduce those bytes. They must never
encode a v2 challenge with the v1 codec.

### 2. Live preconditions (stale-card refusal)

`Request` gains `expect`: up to 16 `{graph, s, p, o | null}` entries, each
checked as an `ExpectedValue`. The curator sets them when it projects an item
into a request, for example:

- `<item> schema:version N`, the item's revision at curation;
- `<item> seeds:status <its value at curation>`, so a closed item no longer matches.

`deck_attest` checks every expectation inside the deck savepoint, after it
acquires the writer and before verifying the assertion. It uses the same
`check_expected` comparison as conditional writes. A failed check refuses with
"stale request: the item changed", and records nothing: no receipt, no consumed
challenge, no Decision. `deck_present` runs the same check, so a stale card is
refused before the operator is even asked to sign.

These are reads only. The Deck still writes only its own protected graph, and
Quipu learns no work-item vocabulary: the predicates come from the curated
request.

### 3. One signed action per request revision

`UNIQUE(request, revision)` stays. Any action (choose, later or comment)
consumes the current revision. Afterwards the curator re-projects the item as
revision `N+1`:

- after a comment, the question is still open;
- after Later, it is still owed;
- after a choice, it is not re-projected unless the item still asks.

This keeps "exactly one decision per question revision", and a replayed or
second submission is still refused.

### 4. The Decision record and the item's disposition

The Decision facts in `urn:quipu:decision-deck` gain three fields:

- `deckAction` (`choose` | `later` | `comment`);
- `deckComment` (when present);
- `deckLabel` (the chosen label).

`forRequest` already links the Decision to the request, and the request's
`forTarget` is the item IRI.

The item's disposition is a **read projection** of these signed facts, not a
second write:

- **answered:** the latest Decision for the item's current request has
  `deckAction = choose`. The item leaves Needs-you. Its work status is
  unchanged, because answering the question does not close the work.
- **later:** the latest Decision has `deckAction = later`. The item shows in
  the visible Later list.
- **waiting:** there is no Decision for the current request, or the latest one
  is `comment`. A comment never resolves a question.
- **Comments:** every `deckComment`, together with its verified actor and
  time, is a visible comment on the item. The work-item client renders it
  alongside ordinary comments.

Because there is one signed write, the decision, the comment and the
disposition can never disagree, and a partial answer cannot exist. This
applies equally to items whose question is only in the title: the curator
projects them with Yes/No labels.

The work-item client (the board CLI and the browser card) reads the deck graph
explicitly. A follow-up in the work-item tool makes `show` render deck
comments.

### 5. Curator transport

`POST /deck/request` exposes `deck_put_request`, behind both switches. The
curator is the **authenticated principal**, never a body field. The route
admits a request only when the bearer's named-credential principal equals
`deck_config.author`.

The curator is a least-privilege named credential that can put deck requests
and nothing else. Issuing that credential is an operator step, and it gates
enablement, not this source.

### 6. Enrollment

The operator's single step:

1. Open the ceremony page on the dedicated HTTPS origin.
2. Approve one passkey creation (`navigator.credentials.create`, with user
   verification required).
3. Read back the short fingerprint the page shows.

The page displays the credential ID, the COSE public key and the fingerprint.
The operator enrolls them with native file access:

```sh
quipu deck configure --origin https://<origin> --rp-id <host> --actor <who> --author <curator>
quipu deck enroll --credential-id <b64url> --cose-key <b64url> --scheme webauthn-es256 --expires-at <epoch>
quipu deck enable
quipu deck list
quipu deck revoke <id>
```

All of these are operator-only CLI commands over the store file, matching the
existing native methods. There is no HTTP route for configuration or
enrollment. The operator confirms that the fingerprint `quipu deck list` prints
equals the one their device showed. A failed or mismatched enrollment is
revoked, not repaired.

### Request and receipt contract for the browser card

- **Present:** `POST /deck/present`
  `{request, revision, credential, action, choice?, comment?}` returns the
  challenge and the raw WebAuthn challenge bytes.
- **Attest:** `POST /deck/attest`
  `{nonce, authenticator_data, client_data_json, signature}` returns the
  receipt: `decision` IRI, `nonce`, `request`, `revision`, `action`, `label`,
  `comment`, verified `actor`, server `recorded_at`, `store_id` (the
  challenge audience) and `tx`.
- **Receipt:** `POST /deck/receipt {nonce}` returns `{found, receipt, tx}`.

The card's `client_operation_id` is the nonce. After a timeout or lost
response, the card calls `/deck/receipt`. It never re-attests, and
re-attesting is refused anyway. The card clears "pending" only after the
receipt is found and the Decision reads back from the deck graph.

Refusals record nothing:

| Refused input | Response |
|---|---|
| stale request, the item changed, or the item is closed | 403 `stale request` |
| an unknown or foreign request | 403 `unknown request` |
| an undeclared label or action | 403 `undeclared choice` |
| a disabled switch | 403 `disabled` |
| a replay | 403 `replayed challenge` |
| a forged assertion | 403 |

## The /update gates, applied here

wu's conditions for this work, and how they apply:

- **Same write gates.** The new `/deck/request` route is a WRITE endpoint in
  `http_auth`, so read-only mode and bearer authorization cover it. It also
  needs its attestation audit. The existing deck routes keep their
  classification.
- **Idempotency key.** The key is the deck actor plus the server nonce. The
  canonical payload hash is the challenge message hash already stored as
  `evidenceHash`. The challenge serialization is the canonical, frozen form.
- **New-state checklist.** No new table: the new fields live in the existing
  JSON `data` columns, and the `deck_*` tables are already classified for
  respace and share completeness. The v1 and v2 golden vectors are frozen.
  The full suite runs.
- **Identity.** Identity is the enrolled credential only. No service principal
  is ever recorded as the human.

## Verification plan

Each "records nothing" case is checked by counting receipts, consumed
challenges and deck facts before and after.

- The v1 golden vector stays byte-identical, and the new v2 vector is frozen.
- A v2 choose, later and comment each verify and record the right action,
  label and comment. A comment leaves the disposition at waiting.
- An undeclared label, a missing comment for `comment`, or an oversized
  comment is refused at present.
- If the item's version moves between present and attest, attest refuses and
  records nothing. The same applies when its status closes. Control: an
  unchanged item is admitted.
- Replay, forgery, wrong origin and expiry stay refused (existing tests).
- A lost response is recovered by `/deck/receipt`, and a second attest is
  refused.
- `/deck/request` admits only when the authenticated principal equals the
  configured author. A body `author` alone is refused.
- Disposition projection fixtures: answered, later, waiting after a comment,
  and re-projection to `N+1`.

## Rollback

- Turn either switch off. Receipts, revocations and consumed challenges stay.
- Removing the source leaves v2 receipts verifiable only by code that knows
  v2. Keep the v2 verifier through any rollback, or keep the decisions
  read-only.
