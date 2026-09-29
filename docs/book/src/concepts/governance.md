# Governance: Policies, Verdicts & the Write Gate

> **Implementation status (2026-08-12):** ✅ **Built** — the Phase-A write gate
> (`src/governance/guard.rs`, wired through `stage_and_guard` in
> `src/store/ops.rs`), the seven governance/overlay MCP tools
> (`src/mcp/governance.rs`), v1 verdict signing (`src/signing.rs`,
> `src/governance/verdict_facts.rs`), authority intersection, and the
> audit/replay machinery. ⬜ Not built: the hank-side structural-policy
> projection (Phase B), `boundary:"transition"` enforcement, the workflow half
> of `require-approval`, and everything from §5 of the signing-plane design
> (shared signing crate, bitemporal key registry, task signing).

Quipu carries a declarative governance vocabulary and an engine that binds it
to the write path. A policy is a fact in the graph like any other; whether it
*enforces* is a runtime decision, and every enforced decision leaves a signed,
auditable verdict behind.

## The vocabulary

An `aegis:Policy` (`shapes/governance.ttl`) names four things: what it governs,
what compliance means, where it binds, and what happens on failure.

```turtle
@prefix aegis: <http://aegis.gastown.local/ontology/> .

aegis:todo-needs-ticket a aegis:Policy ;
    aegis:targets  "aegis:CodeComment" ;
    aegis:claim    "ASK { $target aegis:citesTicket ?t }" ;
    aegis:boundary "action" ;
    aegis:effect   "deny" .
```

- `targets` — the entity **type** the policy applies to.
- `claim` — a SPARQL ASK stating the **compliant** condition; `$target` is
  bound to the entity under evaluation. Satisfied = good.
- `boundary` — `action` (pre-edit/pre-write) or `transition` (workflow step;
  declared but not yet enforced).
- `effect` — `allow | warn | require-approval | deny | escalate | record`.
- `appliesTo` (optional) — repo-relative path globs scoping **where** an
  action-boundary policy binds. Genuinely multi-valued: a policy scoped to
  three globs carries three values, and a consumer accumulates them rather
  than keeping whichever arrived first. Absent means unscoped. Declared with
  `rdfs:range` only — no `rdfs:domain`, so the same term reads identically on
  a future `TextRule` or `Directive`.

An optional `aegis:evidenceProbe` (another ASK: "does evidence exist yet?")
lets the evaluator distinguish `unknown` from `unsatisfied` — no evidence is a
different fact from failing evidence, and neither is collapsed into the other.

### Tripwires: path-boundary policies

A policy carrying `aegis:appliesTo` and **no** selector or predicate is a
**tripwire**: touching the path *is* the crossing, so the claim needs no
evidence beyond the action's own target. `shapes/policies/tripwire.ttl` ships
the catalog — the governed twin of yupana's local
`[[yupana.policy.tripwires]]`, with quipu as the canonical store and yupana
holding only a projected cache. Placement follows SARC Table 3, not
convenience: the `deny` wire is **hard at the PAG** (admissibility is decided
before dispatch — the edit must never land), and the `throttle` wire is
**soft at the PAA** with a declared `aegis:backoffFormula` (it prices a
completed crossing and backs off the actions after it, never the crossing
itself). That formula is not optional decoration: under `validate_placement`
the write gate refuses any policy declaring `effect "throttle"` without an
`aegis:backoffFormula` — a throttle with no backoff is a response nobody can
compile, so the consumer records the crossing and applies no throttle, an
armed-looking wire that prices nothing. There is no soft-at-the-gate wire — a soft constraint has nothing to
price before the action lands. Re-scoping a wire is amending the policy:
the write gate treats an `appliesTo` write as governance-defining and
invalidates the cached policy registry.

## The write-path gate

The single write choke point is `Store::transact_to_graph`. Inside the open
savepoint — datums staged, nothing committed — `stage_and_guard`
(`src/store/ops.rs`) hands the pending post-state to the policy guard
(`PolicyRegistry::build` + `evaluate_write`, `src/governance/guard.rs`). On a
blocking verdict the savepoint rolls back and the write never lands; the store
is byte-identical to before the call.

The registry is built once and cached: every active `boundary:"action"` policy,
indexed by target type. A write's touched entities are intersected with the
governed-type set first, so **a write touching no governed type runs zero
ASKs**; the cache invalidates itself when a transaction writes a
governance-defining fact. Effects split into blocking — `deny`,
`require-approval`, `escalate` block when the claim is unsatisfied (a gate with
no approval channel fails closed rather than passing silently; `escalate`
additionally mints an `aegis:DecisionRequest` for the escalation router) — and
advisory — `allow`, `warn`, `record` never block.

Enforcement is opt-in:

```toml
[quipu.governance]
enforce_on_write = false   # the default
```

Default **off**, mirroring `shacl.validate_on_write`: existing deployments are
unchanged, and turning the gate on is a deliberate configuration act, not a
side effect of upgrading. `validate_placement` (definition-time class↔placement
conformance) and `enforce_authority` (below) are separate flags with the same
opt-in posture.

## On-demand evaluation: `quipu_policy_check`

The read-only half. Given a policy IRI (or an inline `claim`) and a target,
it evaluates the claim over the committed graph and returns a **Verdict**:

```json
{
  "predicate_id": "aegis:todo-needs-ticket",
  "target_ref": "ex:comment-42",
  "outcome": "unsatisfied",
  "evidence_hash": "fnv1a:9c2a41d07be3f118",
  "tier": "committed",
  "verifier": "quipu",
  "verifier_authorized": true,
  "signed": true,
  "signature": "…hex…"
}
```

`outcome` is `satisfied | unsatisfied | unknown`, bound to a reproducible
`evidence_hash` over (predicate, target, `valid_at`, bound claim) — any
verifier re-runs the same ASK over the same committed evidence and must get the
same verdict. Checked, not trusted. `valid_at`/`tx` make the evaluation as-of.

## Verdict signing

Verdicts are attestations, not claims. If the store holds a signing identity
(ed25519 via `ring`, host-file key custody — explicitly v1), it signs the
canonical message `v1|predicate|target|outcome|evidenceHash|tier|verifier`
(`src/signing.rs`). **No signing identity means no persisted verdict, never an
unsigned one** — a bare `satisfied` fact is forgeable by anyone who can write.

Write-gate verdicts are persisted as bitemporal `aegis:Verdict` facts
(`src/governance/verdict_facts.rs`), staged during evaluation and flushed after
the savepoint resolves — so a *denied* write still records its verdict, and
`unknown` is recorded rather than skipped. Their evidence hash seals
attribution: `sha256` over `predicate|target|outcome|writer|chain`, binding the
`aegis:attributedWriter` and `aegis:principalChain` into the signed seal. It is
deliberately *not* a hash of graph state, which has no stable serialisation.

## The Phase-0 root of trust

Trust concentrates in a small, human-owned surface: `aegis:VerifierRegistration`
facts carry each verifier's name, hex `aegis:publicKey`, and the predicates it
`aegis:attests`. A human registers a verifier; quipu never self-registers.

- `quipu_verifier_authorized` — may this verifier attest this predicate?
- `quipu_verdict_verify` — verify a signed verdict's fields against the
  registry. Returns `signature_valid`, `verifier_registered`,
  `verifier_authorized`, and `trusted` = `signature_valid` **AND**
  `verifier_authorized` — the one property a consumer should gate on.

Verification is currently latest-only; the bitemporal key registry (rotation
with as-of re-verification) is designed but not built.

## Transition signatures at the write gate

Shuttle signs every workflow transition with the performing agent's ed25519
key over the canonical message
`shuttle-transition-v1|{run}|{step}|{from}|{to}|{at}|{agent}` and exports the
signature as an ordinary `aegis:signature` fact on the
`aegis:TransitionEvent` — re-derivable from the exported facts alone, so
consumers can re-check it (`shuttle verify`). Under
`[quipu.governance] verify_transitions` (default off), quipu re-checks it
**at the write gate** (`src/governance/transition.rs`): a write landing a
`TransitionEvent` that is unsigned, signed by an agent with no
`aegis:VerifierRegistration`, or whose signature does not verify over the
staged fields under any of the agent's registered keys is refused before it
commits — forged and tampered transitions become unwritable rather than
merely detectable. The registry read spans every graph (shuttle's convention
keeps registrations in a never-frozen `dataKind=identity` named graph);
protecting the registry itself from unauthorized writes is `enforce_authority`'s
job, exactly as for signed decisions.

## Authority over graphs

`aegis:Principal` facts hold `aegis:authorityOver` graph IRIs (or `*`). A call
chain's effective authority is the **intersection** of every link's, so
delegation only narrows and an empty intersection refuses
(`src/governance/authority.rs`). Gated by `enforce_authority` (default off) and
inert for callers that present no principal chain. This gates **writes only** —
it is not a read-side confidentiality boundary.

## Audit & replay

The rest of `src/governance/` closes the loop after the fact:
`quipu_audit_check` (`audit.rs`) mechanically checks a recorded trace against
the policy spec — coverage, class↔placement, outcome consistency, attribution —
deterministically, never an LLM call. `replay.rs` measures whether an advisory
rule is ready for promotion to enforcement (liveness, both outcomes,
recoverability). `router.rs` queues `require-approval` escalations as
`DecisionRequest`s with expiry (only an approval permits; a rejection outranks
an approval); `tree.rs`, `inventory.rs`, and `inheritance.rs` reconstruct
attribution, check dispatch-graph coverage, and detect constraint laundering
under delegation.

## See also

- [MCP tools reference](../reference/mcp-tools.md) — the seven governance and
  overlay tools.
- [REST API reference](../reference/rest-api.md) — the mirrored HTTP routes.
- `docs/design/policy-edit-hooks.md` — the write-gate design and its backlog.
- `docs/design/signing-plane.md` — where signing goes next (proposed).

## Post-commit reactions and firing records

`shapes/aegis-ontology.shapes.ttl` defines `aegis:Reaction` and
`aegis:ReactionFiring` for Chaski. These describe post-commit or scheduled
reactions. Loading the schema does not start an evaluator, install a graph
sink, or deliver an alert.

A reaction has one `rdfs:label`, `triggerKind` (`event` or `schedule`),
`condition` (SPARQL `SELECT ?focus`), `severity` (`info`, `warning`, `critical`
or `page`), and string `owner`. It requests one or more `action` values:
`alert`, `page`, `bead`, or `webhook`. The existing string properties
`severity`, `owner`, and `schedule` are reused.

Event reactions use a comma-separated `eventTypes` string; scheduled reactions
use an ISO-8601 duration string in `schedule`, such as `PT5M`. Chaski's current
consumer supports time durations expressed with hours, minutes and seconds.
`enabled` is an optional boolean; Chaski treats omission as enabled. The graph
does not assert that default. A single `rdfs:comment` can explain the rule.
SHACL checks the declared field types, cardinalities and enums. The consumer
must also validate the trigger-specific configuration and query: the shape
alone does not parse SPARQL or duration strings.

A firing records a transition, never each evaluation. It has exactly one
`firedBy` link to a typed Reaction, one IRI `focus`, and one `xsd:dateTime`
`startedAt`. Its stable IRI identifies the machine-emitted record; it does
not require an `rdfs:label`. The rule and its durable firing record are BFO
continuants, separate from the execution that produced the record.
`resolvedAt` is absent while active and set once via `/set` when
the condition resolves. A failed or timed-out query is unknown and must not
resolve an existing firing. A later firing after resolution is a new
occurrence. SHACL does not enforce these temporal rules or timestamp ordering.

### Stable delivery identity

`ReactionFiring` optionally carries one string `eventId`. When present, derive
the firing IRI deterministically from **(reaction label, eventId)** using an
unambiguous encoding, for example a hash of a JSON array containing both
strings. The event ID identifies an item and transition; including the stable
reaction label separates reactions receiving the same event. If an adapter's
event ID does not uniquely identify its focus, it must include that focus in
the identity input too. Do not substitute a send-time random ID.

For a Chaski outbox event, take `startedAt` from the frozen payload's
`observed_at`, never the retry time. Persist the graph-sink payload before its
first send and reuse it unchanged after a crash. Stable identity alone does
not provide idempotence when timestamps or descriptions change between
attempts. Verify receiver state after an uncertain write; an `unchanged`
response alone is not a presence check. Test send-then-crash replay at the sink.
This schema does not itself implement an idempotent receiver or guarantee
exactly-once external actions.

For an adapter that evaluates whether work has become unblocked, `condition`
selects candidate WorkItems with `aegis:blockedOn`; the adapter decides their
verdict. State that interpretation in the reaction's `rdfs:comment`. Ordinary
scheduled transition firings may omit `eventId` and retain a durable occurrence
identity for each transition of the (reaction, focus) pair.
