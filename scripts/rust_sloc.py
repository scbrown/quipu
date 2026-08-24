#!/usr/bin/env python3
r"""Count CODE lines in a Rust source file — a scanner, not a regex.

ian's ruling on aegis-gf3j7 requires a parser-aware count and explicitly forbids regex
stripping, because "strings/macros/nested comments make that unsound". He asked for a pinned
third-party tool. There is none available where this has to run: the CI job that executes the
gate is `setup-python` + `pip install pre-commit` with NO Rust toolchain (ci.yml, job `check`),
so a cargo-based counter would mean adding rustup to a job that is currently seconds of shell.
tokei/scc/cloc are not installed and adding one is a new supply-chain dependency for a line
count.

So this is a hand-written scanner in the language the job already has. It is not a regex, and
it handles exactly the hazards that made regex unsound, each of which has a test:

  * `//` and `/* */`, including NESTED block comments, which Rust permits and most naive
    strippers get wrong
  * doc comments `///`, `//!`, `/** */` — comments for this purpose
  * string literals containing comment markers: `let s = "// not a comment";`
  * RAW strings `r"..."`, `r#"..."#`, `r##"..."##`, and byte strings `b"..."`, `br#"..."#`,
    where the usual escape rules do not apply and `*/` inside is just text
  * char literals `'a'`, `'\''`, `'\u{1F600}'`
  * LIFETIMES `'a`, `&'static` — the classic trap, because a lifetime looks exactly like the
    start of an unterminated char literal and swallows the rest of the file

A line counts as code if any character outside a comment is non-whitespace. A line holding only
the tail of a multi-line string counts as code, because it is part of a code construct.
"""

import sys


def code_lines(src: str) -> int:
    """Number of lines bearing at least one non-whitespace character outside a comment."""
    n = len(src)
    i = 0
    line = 1
    coded = set()
    depth = 0          # nested block-comment depth

    def mark():
        coded.add(line)

    while i < n:
        c = src[i]

        if c == "\n":
            line += 1
            i += 1
            continue

        if depth:                                    # inside /* ... */
            if src.startswith("/*", i):
                depth += 1; i += 2; continue
            if src.startswith("*/", i):
                depth -= 1; i += 2; continue
            i += 1; continue

        if src.startswith("//", i):                  # line comment to EOL
            while i < n and src[i] != "\n":
                i += 1
            continue

        if src.startswith("/*", i):
            depth = 1; i += 2; continue

        # raw / byte strings: r"…", r#"…"#, b"…", br#"…"#
        j = i
        if src[j] in "bB":
            j += 1
        if j < n and src[j] in "rR":
            j += 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1; j += 1
            if j < n and src[j] == '"':
                mark()
                j += 1
                close = '"' + "#" * hashes
                while j < n and not src.startswith(close, j):
                    if src[j] == "\n":
                        line += 1; mark()
                    j += 1
                i = min(n, j + len(close))
                continue

        if c == '"':                                 # ordinary / byte string
            mark()
            i += 1
            while i < n:
                if src[i] == "\\":
                    if src[i + 1: i + 2] == "\n":
                        line += 1
                    i += 2; continue
                if src[i] == '"':
                    i += 1; break
                if src[i] == "\n":
                    line += 1; mark()
                i += 1
            continue

        if c == "'":
            # LIFETIME or char? A char literal is 'x' or '\...'. Anything else is a lifetime,
            # and treating a lifetime as a string start swallows the file.
            if src[i + 1: i + 2] == "\\":
                mark(); i += 2
                while i < n and src[i] != "'":
                    if src[i] == "\\":
                        i += 1
                    i += 1
                i += 1
                continue
            if src[i + 2: i + 3] == "'":             # 'x'
                mark(); i += 3; continue
            mark(); i += 1; continue                 # lifetime: consume the quote only

        if not c.isspace():
            mark()
        i += 1

    return len(coded)


def _count(path: str) -> int:
    with open(path, encoding="utf-8", errors="replace") as fh:
        return code_lines(fh.read())


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "--batch":
        # "<n> <path>" per line, for a caller that wants one interpreter start.
        for path in args[1:]:
            print(f"{_count(path)} {path}")
    else:
        for path in args:
            print(_count(path))
