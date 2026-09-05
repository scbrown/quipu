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

**Creating that prerelease fires `crates.yml`.** `release: [published]` fires for prereleases as
well as real ones, so step 2 triggers the crates.io publish lane. The publish job refuses a
prerelease outright, and since aegis-pb4rzi it *also* refuses any ref named like a rehearsal
(`rehearsal-*`, `*-rehearsal`, `test-*`) and any version that is not the one the run explicitly
said it intended to publish. Without those, `cargo publish` would try to publish whatever version
`Cargo.toml` currently holds, under a tag nobody intends to ship.

Measured on 2026-09-05: the rehearsal at `rehearsal-wasm-20260905-0951` fired run `33959092337`,
which failed **only because crates.io Trusted Publishing is unconfigured**. That is worth stating
plainly, because it is the reason the guards above are not optional: a procedure whose safety
depends on a different system being broken is not safe, it is untested — and it would have expired
silently the moment Trusted Publishing was configured. Every refusal in
`scripts/ci/crates-publish-guard.sh` holds with Trusted Publishing fully working, and
`scripts/ci/crates-publish-guard.sh --selftest` demonstrates each of them (plus a control arm, so
the suite can tell a correctly strict guard from a uniformly broken one). CI runs that selftest in
`Pre-commit checks`, which gates `release-correctness`.

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

A release **does** now publish `quipu-ai` to crates.io, but not from `crates.yml`. The real lane is
the `crates` job in `release.yml`, added for aegis-pb4rzi; `crates.yml` is the manual lane only.

**Why the publish moved.** `crates.yml` triggers on `release: [published]`, but a release created
by a workflow using `GITHUB_TOKEN` does not trigger further workflows, and release-plz creates the
release that way. Measured 2026-09-05 as a controlled pair: v0.3.34's release produced **no**
`crates.yml` run at all, while a prerelease created by hand on the same day **did**. The cost of
that was six versions: crates.io served 0.3.27 from 2026-08-27 while the repo shipped through
0.3.33, so `cargo add quipu-ai` silently installed nine-day-old code. A missing crate fails loudly;
a stale one succeeds silently, which is why nobody noticed.

Running the publish as a job in `release.yml` removes the cross-workflow event entirely — there is
nothing left to be swallowed. It runs after the wasm and binary asset jobs and only if both
succeeded, because a crates.io version, once published, is permanent.

**Do not read a green `crates.yml` run as a publish.** The 2026-09-03 run was `success` and moved
nothing: it was dispatched with `dry_run=true`, and the dry-run branch skips both the
authentication and the publish steps, so it goes green without exercising either. That
indistinguishability is now fixed at both ends — the dry-run branch says in the log that it
published nothing, and the real branch ends by polling crates.io and **failing** if the registry
does not serve the version it was asked to publish.

**Verify a publish by the registry, not by the workflow.** Note that crates.io requires a
`User-Agent`; without one it answers 403, which reads as "blocked" or "absent" rather than "you
forgot a header".

```bash
curl -s -H 'User-Agent: your-name (contact)' \
  https://crates.io/api/v1/crates/quipu-ai | jq -r '.crate.max_version'
```

The crate is published as `quipu-ai`. The `quipu` name on crates.io belongs to an unrelated
post-quantum cryptography library.
