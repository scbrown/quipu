#!/usr/bin/env python3
"""Is a dataset homogeneous enough to measure SCALING on? Ask before you ingest.

A scaling curve holds the workload constant and varies store size. If the source
changes character partway through, the curve measures the SOURCE and reads as a
property of the store -- and nothing about the resulting number says so.

That is not hypothetical. The WatDiv 100M archive was used for exactly this and
produced a confident "ingest rate halves as the store grows", which survived three
refuted hypotheses before the cause was found in the input (aegis-3sau5a). The
follow-up prescribed loading a homogeneous slice by stopping at 6.0M triples --
and that slice is not homogeneous either (aegis-aoib92): first-seen terms per
triple fall monotonically by 1.47x across it, while predicate count and bytes per
line stay flat. Both cheap eyeball checks pass on a source that varies by half.

So the check has to be the quantity that actually drives ingest cost: NEW terms
per triple, counted against everything seen so far.

    zcat data.nt.gz | python3 source_homogeneity.py --band 250000

WHY CUMULATIVE FIRST-SEEN AND NOT A ROLLING WINDOW
---------------------------------------------------
A rolling window over this signal is what turned one sharp step into a gradual
decay and invented a peak that was then explained three times. Fixed bands with a
cumulative term set have no such smoothing: each band reports the terms THAT band
introduced to a store that has seen every band before it, which is what an ingest
actually pays for.

WHY DIGESTS AND NOT THE TERM STRINGS
-------------------------------------
The strings for a few million triples are hundreds of MB and the set is asked only
whether it has seen a term before. An 8-byte digest answers that; at 3M terms the
collision probability is about 2e-7, which is far below the precision of any
conclusion drawn from the output.
"""

from __future__ import annotations

import argparse
import hashlib
import sys


def bands(stream, band: int, limit: int | None = None):
    """Yield one summary per `band` lines of an N-Triples stream.

    `stream` is binary. Terms are split on ANY whitespace: WatDiv's archives are
    TAB-separated, and a space-only splitter silently matches nothing and reports
    a confident zero for every band.
    """
    seen: set[bytes] = set()
    digest = hashlib.blake2b
    new = nbytes = n = lo = 0
    preds: set[bytes] = set()

    for raw in stream:
        n += 1
        nbytes += len(raw)
        line = raw.rstrip()
        if line.endswith(b"."):
            line = line[:-1].rstrip()
        parts = line.split(None, 2)
        if len(parts) == 3:
            subject, predicate, obj = parts
            preds.add(predicate)
            for term in (subject, predicate, obj):
                d = digest(term, digest_size=8).digest()
                if d not in seen:
                    seen.add(d)
                    new += 1
        if n % band == 0:
            yield _row(lo, n, new, preds, nbytes, len(seen))
            lo, new, nbytes, preds = n, 0, 0, set()
        if limit is not None and n >= limit:
            break
    if n > lo:
        yield _row(lo, n, new, preds, nbytes, len(seen))


def _row(lo, hi, new, preds, nbytes, cum):
    span = hi - lo
    return {
        "band_lo": lo,
        "band_hi": hi,
        "triples": span,
        "new_terms": new,
        "new_per_triple": new / span,
        "distinct_predicates": len(preds),
        "bytes_per_line": nbytes / span,
        "cumulative_terms": cum,
    }


def verdict(rows, tolerance: float) -> tuple[bool, str]:
    """Homogeneous iff new-terms-per-triple stays inside `tolerance` of its median.

    Reported as a RATIO of max to min, not a variance: the failure that matters is
    a sweep across a range, and a 1.47x sweep is what a scaling curve cannot
    survive regardless of how smoothly it got there.
    """
    vals = [r["new_per_triple"] for r in rows]
    if len(vals) < 2:
        return True, "fewer than two bands -- nothing to compare"
    lo, hi = min(vals), max(vals)
    if lo <= 0:
        return False, "a band introduced no new terms; the ratio is undefined"
    spread = hi / lo
    ok = spread <= 1 + tolerance
    return ok, (
        f"new terms/triple ranges {lo:.4f}-{hi:.4f} across {len(vals)} bands "
        f"= {spread:.3f}x (tolerance {1 + tolerance:.3f}x)"
    )


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--band", type=int, default=250_000)
    p.add_argument("--limit", type=int, default=None)
    p.add_argument(
        "--tolerance",
        type=float,
        default=0.10,
        help="fractional spread in new-terms-per-triple still called homogeneous "
        "(default 0.10, i.e. max/min within 1.10x)",
    )
    args = p.parse_args(argv)

    rows = []
    cols = ("band_lo", "band_hi", "triples", "new_terms", "new_per_triple",
            "distinct_predicates", "bytes_per_line", "cumulative_terms")
    print("\t".join(cols))
    for row in bands(sys.stdin.buffer, args.band, args.limit):
        rows.append(row)
        print("\t".join(
            f"{row[c]:.4f}" if isinstance(row[c], float) else str(row[c]) for c in cols
        ), flush=True)

    ok, why = verdict(rows, args.tolerance)
    print(f"\n{'HOMOGENEOUS' if ok else 'NOT HOMOGENEOUS'}: {why}", file=sys.stderr)
    if not ok:
        print(
            "A scaling curve over this range measures the SOURCE, not the store. "
            "Take a slice from a flat region instead (watdiv_ingest.py --skip/--limit).",
            file=sys.stderr,
        )
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
