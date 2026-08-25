#!/usr/bin/env python3
"""Update `.beads/issues.jsonl` directly — this repo has no Dolt database.

## Why this exists

`.beads/metadata.json` declares a Dolt backend, but there is no Dolt database:
not in this clone, and — checked with `git ls-remote origin` — no `refs/dolt/*`
on the remote either. So there is no store to hydrate from and nothing to
reconcile against. `bd` 1.2.2 refuses to operate without one ("no beads
database found"), including with `no-db: true` in `.beads/config.yaml`, so `bd`
commands cannot read or write this tracker.

That makes `.beads/issues.jsonl` **the** tracker, not an export of one — which
is what the history has been doing all along: commits titled
`chore(beads): … (jsonl export)` have edited this file directly for weeks. The
sibling repos scbrown/bobbin and scbrown/camayoc reached the same place and
carry the same script.

The managed beads block calls hand-editing the JSONL an anti-pattern. It is —
*when a Dolt store exists*, because then the edit is to a derived artifact and
the next export silently reverts it. With no store there is nothing to derive
from and nothing to revert it, and the warning does not apply.

## Why a script rather than an editor

Every write here is one careless keystroke away from deleting a record, and the
file is one line per issue with no schema enforcement. So this refuses to write
anything that loses information:

  * a record may never disappear
  * notes and comments may only grow (`--notes` on real `bd` REPLACES)
  * a closed issue may not silently reopen

## Serialization

Writes match what already produced this file: `json.dumps` defaults — spaced
separators, and non-ASCII escaped rather than written literally. Measured, not
guessed: 70 of 78 records round-trip byte-for-byte under those settings, versus
26 with `ensure_ascii=False`. Keeping them means a one-record edit is a
one-line diff instead of a whole-file reformat.

## Usage

    scripts/beads-jsonl.py create "Title" [--description ...] [--priority 3] [--type task] [--label pitch]
    scripts/beads-jsonl.py close <id> --reason "..."
    scripts/beads-jsonl.py note  <id> --text   "..."
    scripts/beads-jsonl.py list [--status open]
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
JSONL = REPO / ".beads" / "issues.jsonl"

#: This file's own convention, measured against its existing records: plain
#: `json.dumps` defaults. Matching it keeps diffs to the lines actually changed
#: instead of reformatting all of them.
def dump(record: dict) -> str:
    return json.dumps(record)


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load() -> list[dict]:
    return [json.loads(line) for line in JSONL.read_text().splitlines() if line.strip()]


def save(records: list[dict], before: list[dict]) -> None:
    """Write, but only if nothing was lost."""
    old = {r["id"]: r for r in before}
    new = {r["id"]: r for r in records}

    for lost in sorted(set(old) - set(new)):
        raise SystemExit(f"refusing to write: {lost} would disappear")

    for issue_id in sorted(set(old) & set(new)):
        a, b = old[issue_id], new[issue_id]
        if a.get("status") == "closed" and b.get("status") != "closed":
            raise SystemExit(f"refusing to write: {issue_id} would reopen; that is a human action")
        if len(b.get("notes") or "") < len(a.get("notes") or ""):
            raise SystemExit(f"refusing to write: {issue_id} notes would shrink")
        if len(b.get("comments") or []) < len(a.get("comments") or []):
            raise SystemExit(f"refusing to write: {issue_id} comments would shrink")

    JSONL.write_text("".join(dump(r) + "\n" for r in records))


def find(records: list[dict], issue_id: str) -> dict:
    for record in records:
        if record.get("id") == issue_id:
            return record
    raise SystemExit(f"no such issue: {issue_id}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_close = sub.add_parser("close", help="close an issue with a reason")
    p_close.add_argument("id")
    p_close.add_argument("--reason", required=True)

    p_note = sub.add_parser("note", help="append to an issue's notes")
    p_note.add_argument("id")
    p_note.add_argument("--text", required=True)

    p_list = sub.add_parser("list")
    p_list.add_argument("--status")

    p_create = sub.add_parser("create", help="file a new issue")
    p_create.add_argument("title")
    p_create.add_argument("--description", default="")
    p_create.add_argument("--priority", type=int, default=3)
    p_create.add_argument("--type", dest="issue_type", default="task")
    p_create.add_argument("--label", action="append", default=[],
                          help="repeatable; e.g. --label pitch")

    args = ap.parse_args()
    records = load()
    before = json.loads(json.dumps(records))  # deep copy for the loss check

    if args.cmd == "list":
        for record in records:
            if args.status and record.get("status") != args.status:
                continue
            print(f"{record['id']:<14} {record.get('status', '?'):<11} {record.get('title', '')}")
        return 0

    stamp = now()

    if args.cmd == "create":
        # Suffix from a hash of title+time, collision-checked — matches the
        # {prefix}-{short} shape of every existing id. Creation adds a record
        # and can never lose one, so the save() guard passes trivially.
        import hashlib

        prefix = records[0]["id"].rsplit("-", 1)[0] if records else "issue"
        existing = {r["id"] for r in records}
        digest = hashlib.sha256((args.title + stamp).encode()).hexdigest()
        for i in range(0, len(digest) - 3):
            candidate = f"{prefix}-{digest[i:i + 3]}"
            if candidate not in existing:
                break
        else:
            raise SystemExit("could not derive a fresh id")
        record = {
            "id": candidate,
            "title": args.title,
            "description": args.description,
            "status": "open",
            "priority": args.priority,
            "issue_type": args.issue_type,
            "owner": "noreply@anthropic.com",
            "created_at": stamp,
            "created_by": "Claude",
            "updated_at": stamp,
        }
        if args.label:
            record["labels"] = sorted(args.label)
        records.append(record)
        save(records, before)
        print(f"created {candidate}")
        return 0

    record = find(records, args.id)

    if args.cmd == "close":
        if record.get("status") == "closed":
            print(f"{args.id} is already closed", file=sys.stderr)
            return 1
        record["status"] = "closed"
        record["closed_at"] = stamp
        record["close_reason"] = args.reason
        record["updated_at"] = stamp
        print(f"closed {args.id}")

    elif args.cmd == "note":
        existing = record.get("notes") or ""
        # Append, never replace. The whole point of the guard above.
        record["notes"] = (existing + "\n\n" + args.text).strip() if existing else args.text
        record["updated_at"] = stamp
        print(f"noted {args.id}")

    save(records, before)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
