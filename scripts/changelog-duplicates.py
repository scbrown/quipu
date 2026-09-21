#!/usr/bin/env python3
"""Fail when a CHANGELOG section documents the same thing twice (aegis-fbekec).

WHY THIS EXISTS, and why the existing guard could not do it.

`verify-changelog.sh` compares the set of commit hashes git-cliff attributes to a
release against the set documented in the changelog, and reports `missing: N ·
extra: N`. That is MEMBERSHIP. Membership is blind to multiplicity: a section that
documents every expected commit TWICE has nothing missing and nothing extra, so it
passes. Measured on quipu 0.7.0 (PR #243, head 793c960e): the published section
carried 55 bullet lines of 34 distinct — 21 duplicates and every category heading
twice — and `./scripts/verify-changelog.sh` exited 0 on it.

That release shipped those notes to the GitHub release page and the crates.io
description. The gate was green the whole time, which is why this is a separate
check and not a tweak to the existing comparison: no amount of refining a set
comparison makes it able to count.

It is also why this runs over EVERY section rather than only the newest one. The
0.7.0 breakage was accompanied by a second, independent blind spot — both the
corrector and the verifier resolve "newest section" as the first `## [` heading,
which was a populated `## [Unreleased]`, so they corrected and certified the
section ABOVE the broken one. A duplicate check that inherited that same section
resolution would have inherited the same blind spot and reported 0.7.0 clean.
Checking all sections is immune to which section anyone thinks is newest.

NOT an error here, deliberately: a populated `## [Unreleased]` sitting above a
released section. That is a legitimate, tested state in this repo — pending work
between releases — and `scripts/test-verify-changelog.sh` asserts both tools accept
it. The defect was never the Unreleased section's existence; it was that nothing
counted the bullets in the section being released.

Usage:
  scripts/changelog-duplicates.py [--file CHANGELOG.md] [--stdin] [--section VER]
  scripts/changelog-duplicates.py --selftest

Exit: 0 = no duplicates; 1 = duplicates found; 2 = usage/parse error.
"""
import argparse
import collections
import io
import re
import sys

HEADING_RE = re.compile(r"^## \[([^\]]+)\]")
CATEGORY_RE = re.compile(r"^### +(.+?)\s*$")
BULLET_RE = re.compile(r"^- +\S")


def split_sections(text):
    """[(version, [lines])] in file order. Lines exclude the `## [..]` heading."""
    sections, cur, name = [], None, None
    for line in text.splitlines():
        m = HEADING_RE.match(line)
        if m:
            if name is not None:
                sections.append((name, cur))
            name, cur = m.group(1), []
            continue
        if name is not None:
            cur.append(line)
    if name is not None:
        sections.append((name, cur))
    return sections


def duplicates_in(lines):
    """(duplicate_bullets, duplicate_categories) as [(text, count)], count >= 2.

    Bullets compare on their full text. Every generated bullet carries its commit
    hash, so two identical bullets are the same commit documented twice rather
    than two commits that happen to read alike.
    """
    bullets = [l.strip() for l in lines if BULLET_RE.match(l.strip())]
    cats = []
    for l in lines:
        m = CATEGORY_RE.match(l.strip())
        if m:
            cats.append(m.group(1))
    dup_b = [(t, n) for t, n in collections.Counter(bullets).items() if n > 1]
    dup_c = [(t, n) for t, n in collections.Counter(cats).items() if n > 1]
    return sorted(dup_b), sorted(dup_c)


def check(text, only=None):
    """Report duplicates. Returns exit status."""
    sections = split_sections(text)
    if not sections:
        sys.stderr.write("ERROR: no `## [version]` sections found\n")
        return 2
    bad = 0
    for ver, lines in sections:
        if only is not None and ver != only:
            continue
        dup_b, dup_c = duplicates_in(lines)
        if not dup_b and not dup_c:
            continue
        bad += 1
        total = sum(1 for l in lines if BULLET_RE.match(l.strip()))
        distinct = len({l.strip() for l in lines if BULLET_RE.match(l.strip())})
        sys.stderr.write(
            "FAIL — the [%s] section documents the same entries more than once\n"
            % ver)
        sys.stderr.write("       bullets: %d total, %d distinct, %d duplicated\n"
                         % (total, distinct, total - distinct))
        for t, n in dup_c:
            sys.stderr.write("       category heading x%d: ### %s\n" % (n, t))
        for t, n in dup_b[:10]:
            sys.stderr.write("       x%d %s\n" % (n, t[:100]))
        if len(dup_b) > 10:
            sys.stderr.write("       ... and %d more duplicated bullet(s)\n"
                             % (len(dup_b) - 10))
    if only is not None and not any(v == only for v, _ in sections):
        sys.stderr.write("ERROR: no [%s] section in CHANGELOG\n" % only)
        return 2
    if bad:
        sys.stderr.write(
            "\nA set comparison (missing/extra) cannot see this — it is membership,\n"
            "not multiplicity. Regenerate the section rather than hand-pruning:\n"
            "  scripts/fix-changelog.sh\n")
        return 1
    n = len(sections) if only is None else 1
    print("OK — no duplicated bullets or category headings in %d section(s)." % n)
    return 0


BROKEN = """# Changelog

## [0.7.0] - 2026-09-16

### Added

- *(share)* thing one([aaaaaaa](u))
- *(share)* thing two([bbbbbbb](u))

### Added

- *(share)* thing one([aaaaaaa](u))
- *(share)* thing two([bbbbbbb](u))

## [0.6.0] - 2026-09-13

### Added

- *(share)* older thing([ccccccc](u))
"""

CLEAN = """# Changelog

## [Unreleased]

- pending thing([ddddddd](u))

## [0.7.0] - 2026-09-16

### Added

- *(share)* thing one([aaaaaaa](u))
- *(share)* thing two([bbbbbbb](u))

## [0.6.0] - 2026-09-13

### Added

- *(share)* older thing([ccccccc](u))
"""


def selftest():
    """Prove BOTH directions, and prove the section-resolution immunity."""
    ok = True

    def arm(name, got, want):
        nonlocal ok
        if got == want:
            print("  PASS  %s (exit %d)" % (name, got))
        else:
            print("  FAIL  %s — wanted %d got %d" % (name, want, got))
            ok = False

    arm("duplicated section fails", check(BROKEN), 1)
    arm("clean file passes", check(CLEAN), 0)
    # The 0.7.0 shape: the breakage is NOT in the first heading. A checker that
    # inherited the old "first `## [` heading" resolution would pass this.
    broken_below = CLEAN.replace(
        "- *(share)* thing two([bbbbbbb](u))",
        "- *(share)* thing two([bbbbbbb](u))\n- *(share)* thing one([aaaaaaa](u))")
    arm("duplicate BELOW a populated Unreleased still fails", check(broken_below), 1)
    arm("populated Unreleased above a release is NOT itself an error",
        check(CLEAN, only="Unreleased"), 0)
    arm("scoping to the clean released section passes",
        check(CLEAN, only="0.7.0"), 0)
    arm("scoping to the duplicated section fails", check(BROKEN, only="0.7.0"), 1)
    arm("missing section is an error, not a pass", check(CLEAN, only="9.9.9"), 2)
    arm("no sections at all is an error, not a pass", check("# Changelog\n"), 2)
    print("  selftest: %s" % ("all arms behaved as specified" if ok else "FAILURES"))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(add_help=True)
    ap.add_argument("--file", default="CHANGELOG.md")
    ap.add_argument("--stdin", action="store_true")
    ap.add_argument("--section", default=None,
                    help="only this version (default: every section)")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.stdin:
        text = sys.stdin.read()
    else:
        try:
            text = io.open(a.file, encoding="utf-8").read()
        except OSError as e:
            sys.stderr.write("ERROR: cannot read %s: %s\n" % (a.file, e))
            return 2
    return check(text, only=a.section)


if __name__ == "__main__":
    sys.exit(main())
