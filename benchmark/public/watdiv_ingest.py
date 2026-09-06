#!/usr/bin/env python3
"""Load a PINNED WatDiv dataset through `quipu ingest` and record what happened.

Ported into this repository so the published bulk-ingest figure satisfies the
benchmark page's rule 2 -- every number comes from a version-pinned, checked-in
runner. Until this existed the number was real but not re-derivable here, and the
index row said so.

WHY THE PUBLISHED ARCHIVE AND NOT THE GENERATOR
-----------------------------------------------
WatDiv ships a generator and pre-generated datasets. This uses the PRE-GENERATED
archives: a figure from a locally generated dataset depends on generator version
and seed, so nobody can reproduce it even in principle. The published archive is
the same bytes for everyone, which is what makes a digest pin worth writing down.

Upstream publishes no checksum, so a pin here attests THE BYTES WE BENCHMARKED,
not the bytes upstream intended. That is a weaker claim and it is stated rather
than papered over: the first run records the digest, later runs verify against it,
and a mismatch is a finding rather than a reason to continue.

THROUGHPUT IS A LIVE-FACT DELTA, NEVER THE PARSE COUNT
------------------------------------------------------
`quipu ingest` reports triples the PARSER produced and says so itself. That is not
what was written: a re-ingest of identical content parses everything and writes
nothing. A rate computed from it would publish the cheap half of the work as the
whole, so this reads the store before and after.

THE POPULATION TRAVELS WITH THE NUMBER
---------------------------------------
WatDiv's "10M" archive holds 10,916,457 triples and its "100M" holds 108,997,714.
A rate quoted "at 10M" is wrong by 9% before anything else is checked, so the
ledger records `triples_declared` beside every rate and the report prints them
together.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import time
from pathlib import Path

#: Archives this runner knows. Names only -- digests are pinned in the pins file,
#: because a digest baked into source cannot be recorded on first run.
SCALES = ("10M", "100M", "1000M")
DEFAULT_BASE_URL = "https://dsg.uwaterloo.ca/watdiv"

#: A run on a busy host is not a measurement. Recorded in every row so a reader
#: can judge it, and used to mark the row invalid rather than to refuse the run --
#: an aborted run teaches nothing, a labelled one teaches what it can.
BUSY_LOAD_FRACTION = 0.5


def load_average() -> float:
    return os.getloadavg()[0]


def cpu_count() -> int:
    return os.cpu_count() or 1


def free_bytes(path: str = "/") -> int:
    st = os.statvfs(path)
    return st.f_bavail * st.f_frsize


def used_fraction(path: str = "/") -> float:
    st = os.statvfs(path)
    total = st.f_blocks * st.f_frsize
    avail = st.f_bavail * st.f_frsize
    used = total - avail
    return used / total if total else 1.0


def live_facts(db: Path) -> int:
    """Facts currently asserted, read from the store itself.

    Returns -1 when the store cannot be read, which a caller must treat as
    UNKNOWN rather than zero -- a delta computed from a silent zero would report
    a successful load as having written nothing.
    """
    try:
        conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        try:
            return conn.execute(
                "SELECT COUNT(*) FROM facts WHERE op = 1 AND valid_to IS NULL"
            ).fetchone()[0]
        finally:
            conn.close()
    except sqlite3.Error:
        return -1


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(chunk), b""):
            digest.update(block)
    return digest.hexdigest()


def read_pins(pins: Path) -> dict[str, str]:
    if not pins.exists():
        return {}
    out: dict[str, str] = {}
    for line in pins.read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) >= 2:
            out[parts[0]] = parts[1]
    return out


def verify_or_record_pin(pins: Path, name: str, digest: str) -> str:
    """Verify an archive against its pin, or record one on first sight.

    Returns "verified" or "recorded". A MISMATCH raises: the upstream artifact
    changed under us, which is a finding and not a reason to carry on.
    """
    pinned = read_pins(pins).get(name)
    if pinned is None:
        pins.parent.mkdir(parents=True, exist_ok=True)
        with pins.open("a") as handle:
            handle.write(f"{name}\t{digest}\tupstream publishes no checksum\n")
        return "recorded"
    if pinned != digest:
        raise SystemExit(
            f"PIN MISMATCH for {name}\n  pinned:     {pinned}\n  measured:   {digest}\n"
            "The upstream artifact changed under us. That is a FINDING, not a reason to continue."
        )
    return "verified"


def stream_source(archive: Path):
    """Yield the archive's .nt member as a binary stream, never unpacking it.

    At the 100M scale the extracted source is ~15.6 GB. Materialising it doubles
    the peak footprint of a transient whose whole design is to leave nothing
    behind, so it is streamed instead.
    """
    tar = tarfile.open(archive, "r:*")
    member = next((m for m in tar if m.name.endswith(".nt")), None)
    if member is None:
        tar.close()
        raise SystemExit(f"no .nt member inside {archive}")
    handle = tar.extractfile(member)
    if handle is None:
        tar.close()
        raise SystemExit(f"cannot read {member.name} from {archive}")
    return tar, member.name, handle


def measure_source(archive: Path) -> tuple[int, str, int]:
    """Count triples, digest the bytes and size the source WITHOUT unpacking.

    A separate pass from the ingest on purpose. Computing the declaration from
    the same stream the loader consumes would be declaring what was just read,
    which agrees with anything and is not a declaration.
    """
    tar, _name, handle = stream_source(archive)
    try:
        digest = hashlib.sha256()
        triples = 0
        size = 0
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
            size += len(block)
            triples += block.count(b"\n")
        return triples, digest.hexdigest(), size
    finally:
        tar.close()


def build_row(
    *,
    scale: str,
    archive_url: str,
    archive_sha: str,
    source_sha: str,
    triples: int,
    source_bytes: int,
    quipu_bin: str,
    quipu_version: str,
    chunk: int,
    timestamp: str,
    exit_code: int,
    seconds: float,
    facts_before: int,
    facts_after: int,
    store_bytes: int,
    load1: float,
    ncpu: int,
) -> dict:
    written = facts_after - max(facts_before, 0)
    busy = load1 >= ncpu * BUSY_LOAD_FRACTION
    reasons = []
    if exit_code != 0:
        reasons.append(f"exit {exit_code}")
    if busy:
        reasons.append(
            f"host load {load1:.2f} on {ncpu} cpus -- CONTENDED; this rate is a lower "
            "bound under contention, not a measurement"
        )
    return {
        "benchmark": "WatDiv bulk ingest",
        "scale": scale,
        "source": {
            "url": archive_url,
            "archive_sha256": archive_sha,
            "nt_sha256": source_sha,
            "triples_declared": triples,
            "bytes": source_bytes,
        },
        "quipu": {
            "binary": quipu_bin,
            "version": quipu_version,
            "chunk": chunk,
            "timestamp": timestamp,
        },
        "host": {"load_avg_1min": round(load1, 2), "ncpu": ncpu},
        "result": {
            "exit": exit_code,
            "seconds": round(seconds, 2),
            "live_facts_before": facts_before,
            "live_facts_after": facts_after,
            "live_fact_delta": written,
            "store_bytes": store_bytes,
        },
        # From the live-fact delta, NEVER the loader's parse count.
        "throughput_facts_per_sec": (round(written / seconds, 1) if seconds > 0 else None),
        "valid_result": not reasons,
        "invalid_reason": "; ".join(reasons) or None,
        "note": (
            "throughput is a before/after delta of live facts; the ingest verb prints a "
            "PARSE count and says so itself"
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scale", choices=SCALES, default="10M")
    parser.add_argument("--archive", type=Path, required=True, help="the .tar.bz2, already fetched")
    parser.add_argument("--quipu", required=True, help="path to a release quipu with `ingest`")
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--chunk", type=int, default=50_000)
    parser.add_argument(
        "--timestamp",
        default="2026-01-01T00:00:00Z",
        help="CONSTANT and supplied: two runs of one pinned dataset must produce the same store",
    )
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--min-free-gb", type=int, default=60)
    parser.add_argument("--keep", action="store_true", help="leave the store behind")
    args = parser.parse_args(argv)

    if free_bytes() < args.min_free_gb * 1_000_000_000:
        raise SystemExit(
            f"PRECHECK FAILED: {free_bytes() / 1e9:.1f} GB free, need >= {args.min_free_gb}"
        )

    archive_sha = sha256_file(args.archive)
    pin_state = verify_or_record_pin(args.pins, args.archive.name, archive_sha)
    triples, source_sha, source_bytes = measure_source(args.archive)
    print(f"source: {triples} triples, {source_bytes} bytes, sha256 {source_sha} (pin {pin_state})")

    version = subprocess.run(
        [args.quipu, "--version"], capture_output=True, text=True
    ).stdout.strip()

    if args.db.exists():
        args.db.unlink()
    before = live_facts(args.db)

    tar, _name, handle = stream_source(args.archive)
    started = time.monotonic()
    try:
        proc = subprocess.Popen(
            [
                args.quipu, "ingest", "/dev/stdin",
                "--graph", f"http://quipu.invalid/watdiv/{args.scale}",
                "--timestamp", args.timestamp,
                "--declare-count", str(triples),
                "--declare-sha256", source_sha,
                "--format", "nt",
                "--chunk", str(args.chunk),
                "--db", str(args.db),
            ],
            stdin=handle,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        _out, err = proc.communicate()
        code = proc.returncode
    finally:
        tar.close()
    seconds = time.monotonic() - started
    if code != 0:
        print(err.strip()[:500], file=sys.stderr)

    after = live_facts(args.db)
    store_bytes = sum(
        p.stat().st_size for p in args.db.parent.glob(args.db.name + "*") if p.is_file()
    )

    row = build_row(
        scale=args.scale,
        archive_url=f"{args.base_url}/{args.archive.name}",
        archive_sha=archive_sha,
        source_sha=source_sha,
        triples=triples,
        source_bytes=source_bytes,
        quipu_bin=args.quipu,
        quipu_version=version,
        chunk=args.chunk,
        timestamp=args.timestamp,
        exit_code=code,
        seconds=seconds,
        facts_before=before,
        facts_after=after,
        store_bytes=store_bytes,
        load1=load_average(),
        ncpu=cpu_count(),
    )

    # LEDGER BEFORE CLEANUP, so a failed run still leaves a record.
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("a") as handle_out:
        handle_out.write(json.dumps(row) + "\n")
    print(json.dumps(row, indent=2))

    if not args.keep:
        for path in args.db.parent.glob(args.db.name + "*"):
            if path.is_file():
                path.unlink()
    return code


if __name__ == "__main__":
    raise SystemExit(main())
