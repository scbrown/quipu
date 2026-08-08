# Census-in-the-wild — a real hank trace through the real audit

Census is synthetic by construction (that is what makes it an oracle);
this directory bounds its external-validity gap with a trace recorded
from the real governed writer and replayed through the same audit.

## The recording

`hank-pre-edit.jsonl` — five Pre-Action Gate decisions recorded by
**hank's actual pre-edit guard** (hank `aee6beb`, `hank hook pre-edit
--tenant polecat`, `mode = "enforce"`), driving edits against a
four-file project where `leaf()` has three callers and the `polecat`
scope allows `max_impacted_files = 1`:

- two **denies** — editing `leaf.rs` impacts three callers, beyond the
  scope's blast radius (`max_impacted_files` unsatisfied → blocked);
- three **allows** — single-caller edits inside the radius.

No hand-editing; the spool is byte-for-byte what
`$HANK_METRICS_PATH` received.

## The replay

```bash
quipu knot shapes/policies/treesitter.ttl --db /tmp/wild.db
quipu audit benchmark/census/wild/hank-pre-edit.jsonl --db /tmp/wild.db
```

## The result (recorded 2026-08-08)

```text
T ⊭ Σ: 2 violation(s), 6 incompleteness(es) over 5 record(s) against
2 constraint(s); 0 line(s) unreadable
```

- **Violation ×2** — the trace cites `max_impacted_files`, which is not
  an action-boundary `aegis:Policy` in Σ: a locally-configured rule
  enforcing outside the specification. The audit's advice is the whole
  thesis in one line: *author it in quipu so it can be audited, or stop
  enforcing it.*
- **Incompleteness ×2 (coverage)** — Σ's `todo-needs-ticket` and
  `no-ticket-in-comment` were never exercised in this window.
- **Incompleteness ×2 (placement)** — the config-file rule declares no
  `constraintClass` / `verificationPoint`.
- **Incompleteness ×2 (attribution)** — no principal chain; SARC §9.3's
  intersection is unavailable to the audit.

## What this demonstrates

The synthetic Census plants defects and confirms they are caught; the
wild trace shows the audit doing the same work **unstaged**: a real
enforcement gap (config-file policy vs authored Σ) surfaced as a
violation with a remediation, and every under-determination reported as
incompleteness rather than silently passed or misreported as a
violation. It also shows the audit's honesty boundary: it cannot
manufacture attribution or placement the runtime never recorded.
