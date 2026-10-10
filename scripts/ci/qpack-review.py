#!/usr/bin/env python3
"""Review every qpack a pull request changes (aegis-fxpbys.1, M2).

For each pack directory touched between the merge base and the PR head, the
two versions are materialized from Git and passed to

    quipu share diff <old> <new> --report --fail-on-introduced

The reports are written to the job summary (the check-run markdown) and to an
optional comment body file. Exit status:

    0  every pack reviewed, no introduced SHACL violation (or no pack changed)
    1  at least one pack INTRODUCES a SHACL violation, or a report could not be
       computed. An unevaluable report is red, never a silent pass.

A pack is a directory holding `export.nt` or `payload.nq` AND `manifest.json`
or `manifest.ttl` at the merge base or at the head. Loose N-Triples files and
test fixtures without a manifest are not packs and are not reviewed.
"""
import argparse
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath

PAYLOADS = ("export.nt", "payload.nq")
MANIFESTS = ("manifest.json", "manifest.ttl")
PACK_FILES = PAYLOADS + MANIFESTS + ("shapes.ttl", "decisions.json")
MARKER = "<!-- quipu-qpack-review -->"
# GitHub caps a step summary at 1 MiB and a comment body at 65536 characters.
SUMMARY_LIMIT = 900_000
COMMENT_LIMIT = 60_000


def annotation(level, title, message):
    """A workflow command; its data must escape %, CR and LF."""
    esc = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::{level} title={title}::{esc}")


def git(repo, *args, check=True):
    out = subprocess.run(["git", "-C", str(repo), *args], capture_output=True)
    if check and out.returncode != 0:
        raise SystemExit(f"git {' '.join(args)}: {out.stderr.decode().strip()}")
    return out


def names(repo, ref, directory):
    """File names directly inside `directory` at `ref` (regular blobs only)."""
    out = git(repo, "ls-tree", "-z", ref, "--", f"{directory}/").stdout.decode()
    found = set()
    for entry in filter(None, out.split("\0")):
        meta, path = entry.split("\t", 1)
        if meta.split()[1] == "blob":
            found.add(PurePosixPath(path).name)
    return found


def is_pack(files):
    return bool(files & set(PAYLOADS)) and bool(files & set(MANIFESTS))


def materialize(repo, ref, directory, files, target, decisions):
    target.mkdir(parents=True)
    wanted = [f for f in ("export.nt", "payload.nq", "shapes.ttl") if f in files]
    if decisions:
        wanted.append("decisions.json")
    for name in wanted:
        blob = git(repo, "show", f"{ref}:{directory}/{name}").stdout
        (target / name).write_bytes(blob)
    if not files & set(PAYLOADS):
        # The pack does not exist on this side: review against an empty graph.
        (target / "export.nt").write_bytes(b"")


def blob_id(repo, ref, path):
    out = git(repo, "rev-parse", "--verify", "-q", f"{ref}:{path}", check=False)
    return out.stdout.strip() if out.returncode == 0 else None


def review(repo, binary, base, head, directory, work):
    base_files, head_files = names(repo, base, directory), names(repo, head, directory)
    # Only a sidecar this PR adds or changes is rendered; an old one from an
    # earlier merge describes that merge, not this change.
    decisions = "decisions.json" in head_files and blob_id(
        repo, base, f"{directory}/decisions.json"
    ) != blob_id(repo, head, f"{directory}/decisions.json")
    old, new = work / "old", work / "new"
    materialize(repo, base, directory, base_files, old, False)
    materialize(repo, head, directory, head_files, new, decisions)
    out = subprocess.run(
        [binary, "share", "diff", str(old), str(new), "--report", "--fail-on-introduced"],
        capture_output=True,
        text=True,
    )
    return out.returncode, out.stdout, out.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", required=True, help="the PR base commit")
    parser.add_argument("--head", required=True, help="the PR head commit")
    parser.add_argument("--repo", default=".")
    parser.add_argument("--binary", default="target/debug/quipu")
    parser.add_argument("--summary", help="append the markdown here (GITHUB_STEP_SUMMARY)")
    parser.add_argument("--comment-body", help="write a sticky-comment body here")
    args = parser.parse_args()
    repo = Path(args.repo)
    binary = str(Path(args.binary).resolve())

    bases = git(repo, "merge-base", "--all", args.base, args.head).stdout.decode().split()
    if len(bases) != 1:
        raise SystemExit("refusing: the PR has no unique merge base")
    base = bases[0]
    changed = git(repo, "diff", "--name-only", "-z", base, args.head).stdout.decode()
    candidates = sorted(
        {
            str(PurePosixPath(p).parent)
            for p in filter(None, changed.split("\0"))
            if PurePosixPath(p).name in PACK_FILES or p.endswith((".nt", ".nq"))
        }
    )
    packs = [
        d
        for d in candidates
        if is_pack(names(repo, base, d)) or is_pack(names(repo, args.head, d))
    ]

    parts = [MARKER, "# qpack review", ""]
    failed = []
    deleted = 0
    if not packs:
        parts.append(
            "No qpack changed in this pull request. A pack is a directory holding "
            "`export.nt` or `payload.nq` and `manifest.json` or `manifest.ttl`."
        )
        if candidates:
            parts.append("\nPack-like files changed outside any pack: " + ", ".join(
                f"`{d}`" for d in candidates))
    for directory in packs:
        head_files = names(repo, args.head, directory)
        if not head_files.intersection(PACK_FILES):
            # Only a proven absence of ALL artifact files is a whole deletion.
            # Missing validation on a surviving pack must use the strict gate.
            parts += [f"## `{directory}`", "",
                      "**Deleted pack:** no payload, manifest, shapes or decisions remain at the head. "
                      "New-head SHACL validation is not applicable.", ""]
            deleted += 1
            continue
        if not is_pack(head_files):
            parts += [f"## `{directory}`", "",
                      "**Incomplete surviving pack:** payload and manifest are required.", ""]
            failed.append(directory)
            continue
        with tempfile.TemporaryDirectory() as tmp:
            code, report, err = review(repo, binary, base, args.head, directory, Path(tmp))
        parts += [f"## `{directory}`", ""]
        if code in (0, 3):
            parts.append(report.rstrip())
            title = report.rstrip().splitlines()[-1] if report.strip() else "no report"
            annotation("error" if code == 3 else "notice", f"qpack {directory}", title)
            if code == 3:
                failed.append(directory)
        else:
            parts.append(f"**Report could not be computed** (exit {code}):\n\n```text\n{err.strip()}\n```")
            annotation("error", f"qpack {directory}", f"report could not be computed: {err.strip()}")
            failed.append(directory)
        parts.append("")
    verdict = (
        f"**Red:** {len(failed)} pack(s) introduce SHACL violations or could not be reviewed: "
        + ", ".join(f"`{d}`" for d in failed)
        if failed
        else f"**Green:** {len(packs) - deleted} pack(s) reviewed, no introduced SHACL violations; "
             f"{deleted} whole pack deletion(s) explicitly identified."
    )
    parts += ["---", verdict, ""]
    body = "\n".join(parts)

    if args.summary:
        text = body if len(body) <= SUMMARY_LIMIT else body[:SUMMARY_LIMIT] + "\n\n(truncated)\n"
        with open(args.summary, "a", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(body)
    if args.comment_body:
        text = body if len(body) <= COMMENT_LIMIT else (
            body[:COMMENT_LIMIT] + "\n\n(truncated: the job summary has the full report)\n")
        Path(args.comment_body).write_text(text, encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
