# The agent arm (extension) — an external writer against the gate

The scripted Census proves the gate refuses planted defects; this arm
lets a real agent be the writer, and measures the loop the thesis
predicts: **write → read the structured refusal → revise → resubmit.**

## Protocol

A recording is a JSONL file of write actions (schema in
`examples/census/agent.rs`). Replay it through the gated store:

```bash
just bench census --recording benchmark/census/agent/attempt1.jsonl
```

Each action becomes a `CEN-AG.n` manifest probe carrying the gate's
verbatim feedback — the only channel the writer gets.

## The recorded demonstration (one LLM writer, two attempts)

The committed fixtures are a genuine single-model run: an LLM agent
(Claude) was given the recording task cold — "record households h1–h3,
one resident placement, one weekly summary; you write as amaru" — and
authored `attempt1.jsonl` without seeing Σ.

**Attempt 1: 2 accepted, 3 refused.** The three refusals, verbatim
categories: a tally with no label (`tally-label` policy), a write into
district-south that amaru's authority does not reach (empty
intersection, with the chain's actual holdings in the message), and a
summary using the fabricated predicate `vocab:populationEstimate`
(closed-vocabulary policy).

**Attempt 2** (`attempt2.jsonl`) revised strictly from the refusal
text — label added, h3 routed through chaski who holds south, the
declared `vocab:households` in place of the invention:
**5/5 accepted.** Final graph defects: zero.

## What this does and does not show

- It shows the retry loop converging in one revision with no human in
  the loop and no access to Σ beyond the refusal messages — the
  "agents bear the cost of strictness" mechanism, live.
- It is a single model, a single task, n=1 — an existence
  demonstration, not a rate. The controlled version needs camayoc's
  competency-question runner as the quality oracle (not yet built) and
  multiple tasks/models; that experiment stays future work, and the
  paper reports this arm as an extension.
