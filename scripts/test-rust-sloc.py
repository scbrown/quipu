#!/usr/bin/env python3
r"""Hazard cases for the Rust code-line scanner (aegis-gf3j7).

ian's ruling forbids regex stripping because "strings/macros/nested comments make that
unsound". scripts/check-file-size.sh claims each of those hazards is tested here, so this file
is what makes that claim true rather than reassuring.

The lifetime case is the one that matters most: `&'a str` looks exactly like the start of an
unterminated char literal, and a scanner that treats it as one swallows the rest of the file
and reports a wildly low count — silently, and in the direction that PASSES the gate.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from rust_sloc import code_lines  # noqa: E402

CASES = [
    ("plain code", "let x = 1;\nlet y = 2;\n", 2),
    ("line comment", "// c\nlet x=1;\n", 1),
    ("doc comments", "/// d\n//! m\nlet x=1;\n", 1),
    ("block comment", "/* a\nb */\nlet x=1;\n", 1),
    ("NESTED block comment", "/* a /* b */ still */\nlet x=1;\n", 1),
    ("comment marker inside a string", 'let s = "// not a comment";\n', 1),
    ("raw string containing */", 'let s = r#"*/ /* "#;\n', 1),
    ("byte raw string", 'let s = br#"//"#;\n', 1),
    ("char literal", "let c = 'a';\n", 1),
    ("escaped char literal", "let c = '\\'';\n", 1),
    ("LIFETIME, not a char", "fn f<'a>(x: &'a str) -> &'static str { x }\n", 1),
    ("lifetime then a real comment", "fn f<'a>() {} // c\nlet z=1;\n", 2),
    ("multi-line string counts both", 'let s = "a\nb";\n', 2),
    ("blank lines are not code", "let x=1;\n\n\n\nlet y=2;\n", 2),
    ("a file of only comments is 0", "// a\n/* b */\n//! c\n", 0),
    ("trailing code after a block comment", "/* x */ let y = 1;\n", 1),
]


def main() -> int:
    bad = 0
    print("rust code-line scanner:")
    for name, src, want in CASES:
        got = code_lines(src)
        if got == want:
            print(f"  ok: {name}")
        else:
            print(f"  FAIL: {name} (want {want}, got {got})")
            bad += 1
    print(f"{len(CASES) - bad}/{len(CASES)} passed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
