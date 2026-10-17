# Four-valued rule answers

`rule-query` reads two explicit evidence predicates for one grounded claim.
It does not change SPARQL `ASK`, Datalog negation-as-failure, or materialization.
The support bits are independent:

| Positive | Explicit negative | Truth |
|---|---|---|
| true | false | `supported` |
| false | true | `refuted` |
| true | true | `both` |
| false | false | `neither` |

Only complete scoped reads can establish `neither`. A failed read, missing
purpose profile, cap, invalid judgment, or unsupported scope produces
`status: unknown` with `truth: null`. Transport failures are also UNKNOWN to a
consumer. A successful health check cannot establish evidence completeness.
Configured ROOT label floors remain enforced; a quality refusal is UNKNOWN,
not a filtered empty evidence set. Label floors are not access control.

## Request and interfaces

CLI: `quipu rule-query request.json --db existing.db`.
HTTP: `POST /rule_query`. MCP: `quipu_rule_query`.
All use the same native query implementation and response types.

```json
{
  "subject": "urn:claim",
  "positive_predicate": "urn:supported-by",
  "negative_predicate": "urn:refuted-by",
  "object": {"iri": "urn:evidence"},
  "judgment_policy": "urn:policy:referee",
  "witness_limit": 64
}
```

Version 1 supports IRI and plain-string objects (`{"str":"text"}`), current
primary ROOT only. Predicates must differ. IRIs must be absolute. Unknown fields,
named graphs, temporal selectors, and unsupported term variants refuse instead
of selecting another scope. Attached/composed stores are unsupported. The CLI
opens an existing database read-only and does not create or migrate it.
Only ROOT assertions participate. The normal reasoner/OWL companion inferred
graph is not folded into this scope, and the query does not run an engine or
certify derived closure. Use an explicitly composed SPARQL dataset when that
different scope is required.

CLI and direct HTTP JSON decoding reject duplicate fields. MCP arguments have
already become a JSON object in the upstream protocol parser; this tool cannot
recover duplicate keys discarded before its invocation. All adapters reject
undeclared fields in the object they receive.

## Response and completeness

The protocol is `quipu.rule-answer/v1`. A complete response contains `evidence`
and `judgment`. Evidence includes typed `truth`, `positive_supported`,
`negative_supported`, positive/negative witness arrays, stable `store_id`,
transaction watermark, ROOT graph IRI, and scope
`current_primary_root_all_physical_assertions`.
`claim_context` binds the exact grounded claim, its digest, and the expected
judgment policy to the response, so retained results do not depend on an
out-of-band request to identify what was checked.

Each witness retains the asserting transaction, valid-from spelling, and nullable
source. Sources are historical or caller-declared metadata, not authenticated
authority. Physical source owners and legacy string representations participate;
the witness cap (1–256 per side) is conservative across them. Exceeding it yields
UNKNOWN, never a truncated answer. A missing source remains explicit.
Metadata is also bounded before loading strings: 16 KiB per field and 64 KiB
per support side. Exceeding either byte bound yields UNKNOWN.

Support and judgment reads share one SQLite read transaction. Reads do not
intern missing terms, materialize facts, admit verdicts, or modify the ledger.
Witnesses and admitted verdict identifiers have deterministic order. Removing
one side of `both` leaves the other; unrelated claims are unaffected.

Every response says `action_authority: not_granted`. There is no boolean
conversion. Before adopting this answer in a boolean consumer, define and review
an explicit policy for conflict, neither, deferral, and UNKNOWN.

## Separate judgment disposition

Judgment is `none_recorded`, `deferred`, or `resolved`. It is not a fifth truth
value. `none_recorded` means absence from the complete supported current ledger
scope, not absence of judgments elsewhere. An unresolved or unverifiable ledger
is UNKNOWN. Deferral and resolution preserve the support bits, including `both`.
The response names the judgment scope
`current_root_records_with_retained_marker_history`. Retained historical markers
locate withdrawn or altered judgments; retracting a marker cannot turn a deferral
into `none_recorded`. Those records must still verify in their current state or
the answer is UNKNOWN. This is not a certificate of history outside the retained
primary store, and it does not provide historical snapshot selection.

The purpose profile in `shapes/rule-answers.ttl` targets only Decisions carrying
`http://quipu.local/rule#judgmentClaim`. It reuses the governed Decision class;
legacy Decisions outside that marker remain outside the profile. The reviewed
profile must be loaded as `rule-answers` with exact source bytes before the query
can establish judgment completeness. Source presence alone does not load it.
Normal schema review and delivery are required; this query never promotes it.

The marker is SHA256 (lowercase hex) of UTF-8 JSON serialized in this field order:
`subject`, `positive_predicate`, `negative_predicate`, `object`, `graph`.
The graph is `urn:quipu:graph:root`; `object` uses the request's externally tagged
IRI/plain-string form. Native serialization defines the exact escaping. This is
not a claim of JSON Canonicalization Scheme conformance. Policy and witness cap
are excluded, so a record with a mismatching policy is detected rather than
disappearing into absence.

The Decision's `rdf:value` is strict `quipu.rule-judgment/v1` JSON with `protocol`,
the full `claim` above, named `authority`, nonempty `reason`, `disposition`
(`deferred`/`resolved`), and nullable `predecessor`. The existing `decisionPolicy`
must match the request. Resolution's predecessor names the earlier admitted
`verdict` and exact `decision_digest` (`sha256:...`). Earlier records remain.
The existing Decision carrier also retains `outcome: approve`, `by` equal to
the named authority, and `evidenceHash: sha256:<claim-digest>`. Its ordinary
`approve` option endorses the sealed disposition record. These fields satisfy
the existing Decision shape; the new purpose profile does not replace it.

Only an existing admitted sealed Decision counts: the normal seal verifier checks
content, signature, historical registered HumanTier policy authorization, expiry
at admission, and the native nonce-spend/verdict transaction binding. The named
authority must match that verifier. The admitted outcome must be `approve`;
deferred/resolved comes from the sealed JSON, never from an unsealed label.
A copied signature or caller's authorization assertion
cannot supply this proof. No signer registration or authority policy is added.

The reader bounds records, entity bytes and references before invoking the seal
verifier. The purpose profile supports scalar sealed Decision fields, avoiding
unbounded blank-node CBD traversal. Unsupported fields, missing predecessors,
tampering, mismatched policy/claim/authority, orphan resolution, and competing
lifecycle heads yield UNKNOWN. Version 1 supports one deferral and its retained
resolution, not arbitrary judgment histories or reopening.

## Standards comparison

[RIF-FLD Second Edition, sections 3.2 and 3.6](https://www.w3.org/TR/2013/REC-rif-fld-20130205/)
permits multiple truth values and distinguishes symmetric negation from NAF.
The two-bit wire envelope here is an implementation contract, not an asserted
RIF dialect or a RIF-prescribed JSON format.

[LegalRuleML 1.0, sections 4.2.1 and 4.3.4](https://docs.oasis-open.org/legalruleml/legalruleml-core-spec/v1.0/os/legalruleml-core-spec-v1.0-os.html)
discusses conflicting defeasible conclusions and named authority metadata.
This foundation reports both supports; superiority and defeasible adjudication
are separate. Named metadata is distinct from local seal authorization. Neither
standard prescribes this envelope or its judgment lifecycle.
