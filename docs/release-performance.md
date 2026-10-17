# Release-plz cost and the private 0.x exception

Release-plz keeps conventional-commit versioning, the exact-commit CI correctness
gate, release-window coverage checks and the existing ledger-only push filter.
Its automatic API-break hint is disabled **only for `quipu-ai`**, while that
package is 0.x and Release-plz publication is disabled (`publish = false`).
This is not a claim that an API change is compatible. Reviewers must still mark
breaking changes correctly in Conventional Commits.

## Why the cache alone was insufficient

The historical cold check took 1,203 seconds. A dedicated Rust cache and ledger
path filter already existed, but three code-push Release-plz jobs on 2026-10-16
still took 1,321, 1,325 and 1,355 seconds. The first was
[run 38893215505](https://github.com/scbrown/quipu/actions/runs/38893215505): API
compatibility ran from 11:32:00 to 11:52:49Z (1,249 seconds), with a cache miss.
Installing the prebuilt cargo-semver-checks binary was subsecond. The long work
was checking the library, not installing the tool.

The ledger filter works independently: commit 459b598 changes only the two
allowlisted ledger-output locations and has no Release run. Source and normal
CI changes outside those locations still run Release.

## Reversal

**Restore `semver_check = true` (or remove the override) before enabling
publication or reaching 1.x.** `scripts/check-semver-policy.py` runs in source CI
and before Release-plz, refusing those transitions while the exception remains.
The exception cannot be copied to another package or widened to the workspace.
If a package is published outside this automation, its owner must also restore
the check; this local configuration guard cannot observe external publication.

A performance rollback simply restores the check; leave CI, publication
credentials, tags, releases and the ledger filter untouched.

## Acceptance is a measured run

After the source change lands through review, measure the next ordinary **code**
main push using the jobs API: Release-plz wall time should be under about five
minutes versus 1,321–1,355 seconds. Record the exact SHA, run and step timings.
Keep the performance item open until that run completes. A fixture or a green
source PR does not prove the hosted timing. CI correctness wait time is a
separate job and is not removed from release safety by this exception.
