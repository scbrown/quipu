# Schema Evolution

Quipu enforces strict ontology rules via SHACL shapes. When an agent writes
data that fails validation because the schema is too restrictive or missing a
class/property, it can **propose** a schema change instead of asking a human to
edit shapes manually.

## The Proposal Workflow

```text
Agent writes data → SHACL rejects it → Agent submits proposal → Approver accepts/rejects
```

Proposals follow a simple lifecycle: **pending → accepted** or **pending → rejected**.
Every proposal requires an explicit approver — there is no auto-accept.

## Proposal Kinds

| Kind       | Description                                    |
|------------|------------------------------------------------|
| `shape`    | New or updated SHACL shape (Turtle fragment)   |
| `ontology` | OWL axiom change (future)                      |
| `class`    | New RDF class definition                       |
| `property` | New or modified property definition             |

## MCP Tools

### Submit a Proposal

```json
{
  "kind": "shape",
  "target": "PersonShape",
  "diff": "@prefix sh: ... ex:PersonShape a sh:NodeShape ; ...",
  "rationale": "Need email property for contact info",
  "proposer": "agent/data-enricher",
  "trigger_ref": "validation-failure-42"
}
```

Tool: `quipu_propose_schema_change`

### List Proposals

```json
{ "status": "pending" }
```

Tool: `quipu_list_proposals`

### Accept a Proposal

```json
{ "id": 1, "decided_by": "aegis/crew/braino", "note": "Looks good" }
```

Tool: `quipu_accept_proposal`

When a **shape** proposal is accepted, Quipu:

1. Validates the Turtle diff is syntactically correct
2. Verifies it parses as valid SHACL
3. Writes the shape to the `shapes` table
4. Records the approver and timestamp

If the Turtle is invalid, the proposal stays **pending** and an error is returned.

### Reject a Proposal

```json
{ "id": 1, "note": "Too permissive — would allow unconstrained strings" }
```

Tool: `quipu_reject_proposal`

## CLI

```bash
# List all pending proposals
quipu propose list --status pending

# Submit a proposal from a Turtle file
quipu propose submit shape PersonShape shape.ttl --proposer agent/enricher

# Accept
quipu propose accept 1 --note "Approved"

# Reject
quipu propose reject 1 --note "Needs tighter cardinality"
```

## Validation Hints

When `quipu_knot` rejects data due to SHACL violations, the response includes a
`hint` field pointing to `quipu_propose_schema_change`. When
`validate_or_reject` fails, the error message includes the same hint. This gives
agents a clear remediation path instead of a dead-end error.

## Design Notes

- The `proposals` table uses a text `diff` column so that SHACL shape diffs and
  future OWL axiom diffs can coexist. The `kind` column disambiguates.
- Proposals only mutate shape/ontology definitions — they never touch the EAVT
  fact store directly.
- The approver role defaults to `aegis/crew/braino`. A capability-based
  authorization system (`quipu.schema.approve`) is a future concern.
