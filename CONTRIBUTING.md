# Contributing to quipu

## Using Just

This project uses [just](https://github.com/casey/just) as a command runner. **Always prefer `just` commands over raw tool commands** — they're configured with sensible defaults.

```bash
just --list          # Show available commands
just setup           # Install pre-commit hooks
just check           # Run all quality checks
just test            # Run tests (debug mode)
just lint            # Run clippy
just fmt             # Format code
```

## Setup

1. Install [just](https://github.com/casey/just)
2. Install [pre-commit](https://pre-commit.com/)
3. Install doc tooling: `cargo install mdbook mdbook-mermaid`
4. Run `just setup` to install git hooks

**Canonical hook installation:** `just setup` runs `pre-commit install`, which
writes `.git/hooks/pre-commit`. This is the supported path. The `.githooks/`
directory exists only as a fallback for environments without the `pre-commit`
binary — do not use `git config core.hooksPath .githooks` as a primary method.

## Pre-Commit Hooks

This project uses [pre-commit](https://pre-commit.com/) to enforce quality standards. Hooks run automatically on `git commit` and include:

- Trailing whitespace removal
- End-of-file newline
- YAML/JSON validation
- Merge conflict detection
- Markdown linting

To run all hooks manually:

```bash
just check
```

## Quality Gates

All checks must pass before pushing:

```bash
just check           # Pre-commit hooks
just test            # cargo test
just lint            # cargo clippy -- -D warnings
```

**Debug builds only** — never pass `--release`. Debug is fast enough for validation.

### `cargo build` does not compile your tests

Test modules are `#[cfg(test)]`, so `cargo build` never compiles them. A clean build is
therefore **no evidence at all** about a file you just edited if that file is a test module —
it can contain an unclosed delimiter and `cargo build` will have nothing to say.

This matters because "I ran `cargo build`, it's fine" is the natural habit, and it is a check
that passes *because it did not look at the thing*. Measured on 2026-09-05 while resolving a
conflict in `src/align/tests.rs`: `cargo build --no-default-features` was clean over a file with
a brace-balance error that `cargo test` caught immediately.

Use `cargo test` (or `cargo clippy --all-targets`, which does compile test targets) whenever the
edit touched a test module. `--all-targets` is also why the lint gate catches things the build
does not — e.g. a non-snake-case test name.

### After resolving a merge conflict, verify BOTH sides by NAME

Check that specific tests from each side of the conflict are present and passing, by name. **Do
not verify by count.** If a resolution drops one side's hunk and keeps another's, the total can
match while the content is wrong — a matching count is precisely what a lost hunk looks like.

### A red conformance ledger during review is EXPECTED — derive ONCE, last

If your PR touches `CODE_PATHS` — `src/` or `benchmark/public/*.py` — the
**Published results match the ledgers** check will be red until you commit re-derived
ledgers. Re-deriving is a manual chore:

```bash
gh workflow run conformance.yml --ref <your-branch>     # ~5 min
gh run download <run-id> -D /tmp/led
cp /tmp/led/conformance-ledgers-*/*.json benchmark/public/results/
python3 benchmark/public/conformance_report.py          # re-render the page
python3 benchmark/public/conformance_report.py --check   # must be rc=0
git add benchmark/public/results/ docs/book/src/benchmarks/   # ONE commit, both
```

**Do it ONCE, after review has converged. Not when the check first goes red.**

A red *required* check invites you to green it immediately, and that instinct is what
makes this expensive. Every review comment that produces a source change — which is what
a good review produces — invalidates the ledger again, so greening early costs one full
derive *per review round* instead of one per PR. Measured on 2026-09-11: a one-line fix to
a test module, made in response to a correct review finding, staled a ledger that had been
derived twenty minutes earlier.

Two facts make waiting safe, and you need both:

- **A ledger/docs commit does NOT re-stale the ledger.** So committing the ledgers does not
  invalidate them, and there is no deadlock — a natural first guess, and a wrong one that
  sends you hunting a cycle that is not there.
- **A source change usually DOES.** Derive after the last source change, not after the first.

The check is literally a two-point CONTENT diff between the ledger's stamped revision and
`HEAD`, restricted to those paths (`conformance_report.py:668`):

```bash
git diff --name-only <ledger quipu_revision> <head> -- src 'benchmark/public/*.py'
```

It is not "was there a commit touching `CODE_PATHS`", and the difference is slack you can
use: a source change made and then **reverted** within the PR leaves that diff EMPTY, so the
existing ledger still stands and needs no re-derive. Likewise a source file moved and moved
back. If you are unsure whether you owe a derive, run that diff — it is the same question the
gate asks, and it answers in a second.

Commit the ledgers **and** the rendered page together. The gate asks for them in two steps
whose messages read as unrelated failures — `a ledger was NOT derived from the code it
ships with`, then `published results disagree with the ledgers` — but the published page
embeds each ledger's provenance stamp, so re-deriving necessarily stales it. They are two
steps of one chore, and splitting them costs an extra CI round.

Finally, when comparing ledgers by hand, compare the **result field per test id** — never
whole row objects and never file bytes. `duration_ms` lives inside each result row, so a
row-level diff reports ~98 differences on `shacl-core` on every run, and a byte-level diff
makes any two ledger-touching PRs conflict unconditionally. Timing is an observation about
the machine, not about conformance.

## Adding New Dependencies

Heavy or optional dependencies MUST be feature-gated:

```toml
# In Cargo.toml
lancedb = { version = "0.17", optional = true }

[features]
lancedb = ["dep:lancedb"]
```

`cargo build` with default features must always compile without new optional deps.

## Adding New Modules

Follow the existing pattern — extend `Store` via impl blocks in dedicated files:

```rust
// src/my_feature.rs
use crate::store::Store;
use crate::error::Result;

impl Store {
    pub fn my_method(&self) -> Result<()> { ... }
}
```

Then add to `src/lib.rs`:

```rust
#[cfg(feature = "my_feature")]
pub mod my_feature;
```

## Testing

- All existing tests must pass: `just test`
- New functionality must include tests
- Use `Store::open_in_memory()` for test fixtures — no temp files needed
- Feature-gated code needs tests behind `#[cfg(test)]` within the gated module

Example test pattern (from `src/vector.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_test() {
        let store = Store::open_in_memory().unwrap();
        // Store is ready with all tables initialized
    }
}
```

## Documentation

Documentation must build cleanly and pass linting:

```bash
just docs build      # Build the book
just docs serve      # Serve locally with hot reload
just docs lint       # Lint markdown files
just docs check      # Full docs quality gate (lint + build)
```

When making user-facing changes, update the relevant documentation under `docs/book/src/`.

## Releasing

See [docs/RELEASING.md](docs/RELEASING.md).

Merging the `release-plz` PR is the whole normal procedure. The one thing worth knowing
before you need it: if you ever repair a release's assets by dispatching `release.yml`, you
must pass `--ref <tag>`. `gh workflow run` defaults to the default branch, which builds
`main` and publishes it under the tag's filenames with regenerated checksums that match the
wrong build. `assert-tag-is-head` refuses that and prints the correct command.
