# Releasing

## The normal path — nobody dispatches anything

`release-plz` opens a release PR. Merging it to `main` tags the commit it built and the
`release` workflow publishes the GitHub release and its assets. That is the whole procedure.
The tag and the built commit agree by construction, because release-plz tags the very commit
the run is building.

## Repairing a release whose asset upload failed

`release.yml` has a `workflow_dispatch` path that takes a tag. It exists for exactly one
situation: a release whose asset upload failed, re-run against **its own commit**.

```sh
gh workflow run release.yml --ref <tag> -f tag=<tag>
```

**`--ref <tag>` is the load-bearing part, and it is not the default.** The asset jobs check
out with no explicit `ref`, so they build whatever ref the workflow was dispatched on.
`gh workflow run` uses the repository's default branch unless you pass `--ref` — so the
obvious command builds `main` and merely *labels* the output with your tag.

That is not a cosmetic error. Every asset upload passes `--clobber`, both asset jobs run on
a dispatch (the binary tarball, its `.sha256`, and the repository qpack), and the checksum
files are regenerated in the same run — so they agree with the wrong build and a downloader
verifying the release **passes**. The substitution is self-consistent and undetectable, and
a release page is not revertible.

`assert-tag-is-head` refuses that case before any asset is written, and prints the command
above. If you are reading its refusal: you almost certainly omitted `--ref`. Add it. Do not
remove the check to get past it — the check is the only thing standing between a failed
upload and a silently corrupted published release.

An unresolvable tag also refuses, so the guard fails closed.

## Rehearsing the release workflow without touching a real release

A pull request cannot exercise `release.yml`'s asset path — this is not a hypothetical gap:
v0.3.33's release-side wasm job died (`spawnSync .../target/release/quipu ENOENT`) while the
PR-side job was green on the identical commit, because what the PR job exercised was
`ci.yml`'s environment rather than `release.yml`'s.

To exercise it for real:

1. Create a throwaway tag at the commit you want to build.
2. Create a **prerelease** for that tag. Prerelease matters: a deploy lane that resolves the
   latest *published* release must not be able to select it. The dispatch requires the
   release to exist already, so create it before dispatching.
3. `gh workflow run release.yml --ref <throwaway-tag> -f tag=<throwaway-tag>`
4. Confirm the jobs are green and the assets are the ones you expected.
5. Delete the release and the tag.

That tag's commit *is* the commit being built, so `assert-tag-is-head` passes — the guard is
designed to permit exactly this.

This procedure was exercised on 2026-09-05 (run `33959094905`) against a throwaway prerelease at
`8ca4bcd`. It is also what proves the repair form above, because a rehearsal dispatched with
`--ref <tag>` *is* the repair form: the guard printed

```text
tag rehearsal-wasm-20260905-0951 -> 8ca4bcd73f7021c18d7b4b272cdbbb588ffcaa55
this run built                   -> 8ca4bcd73f7021c18d7b4b272cdbbb588ffcaa55
OK: rehearsal-wasm-20260905-0951 is the commit this run built.
```

and all nine steps of the wasm asset job passed, including the shared build and smoke actions and
the upload. That run is the first green execution of the release-side wasm job since it died on
v0.3.33.

## What a release does NOT do

`crates.yml` fires on `release: published` and publishes `quipu-ai` to crates.io using
crates.io Trusted Publishing. It is a separate lane: if it fails, the GitHub release and its
assets are unaffected. Verify a crates.io publish by the version appearing on crates.io, and
**not** by dispatching that workflow with `dry_run=true` — the dry-run branch skips both the
authentication and the publish steps, so it goes green without exercising either.

The crate is published as `quipu-ai`. The `quipu` name on crates.io belongs to an unrelated
post-quantum cryptography library.
