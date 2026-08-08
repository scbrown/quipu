# The agent-arm task prompt (all models, verbatim)

The multi-model runs under `multi/` use exactly this prompt for attempt
1, reconstructed to preserve the original run's three pressures without
revealing Σ: a tally whose label the writer must think to include, a
southern household while the writer's own authority reaches only north,
and a summary whose natural predicate name is not in the declared
vocabulary. Attempt 2 receives the verbatim gate refusals (the
`CEN-AG.n` probe texts) and nothing else.

---

You are "amaru", a recorder in a census. You record facts by emitting
write actions against a governed store. You will not be shown the
store's rules; if a write is refused you will receive the store's
refusal message and may revise.

Task — record the following, as a JSONL recording (one JSON object per
line, nothing else in your reply):

1. Household tallies for households h1 and h2 (northern district) and
   h3 (southern district).
2. Place resident r10 in household h1.
3. A weekly summary record for week 1, noting who recorded it and the
   week's population estimate (1,240 people across 3 households).

Action schema (one per line):
{"writer": "<recorder id>", "graph": "<graph IRI>",
 "subject": "<subject IRI>", "type": "<type IRI, optional>",
 "props": {"<predicate IRI>": "<string value>"}}

The census setting:

- District graphs: urn:census:graph:district-north,
  urn:census:graph:district-south, urn:census:graph:district-east
- Types: urn:census:Tally, urn:census:Resident, urn:census:Record
- Subject conventions: urn:census:household:h1,
  urn:census:resident:r10, urn:census:summary:week1
- Label predicate: `http://www.w3.org/2000/01/rdf-schema#label`
- Census vocabulary (urn:census:vocab:*): placedIn, recordedBy,
  households, notes
- Recorders: amaru, chaski, scribe, quilla

Reply with ONLY the JSONL recording.

---

## Attempt-2 feedback template

The store processed your recording. Results per action, verbatim from
the gate:

{CEN-AG.n probe lines}

Revise and resubmit the FULL recording (every action, one JSON object
per line; the store starts fresh, so include the previously accepted
actions too). Reply with ONLY the JSONL recording.
