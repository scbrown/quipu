# Release PR credential

The release workflow uses two credentials. `release-plz release` retains
`GITHUB_TOKEN`: its output drives the existing artifact and crates publishing
jobs inside `release.yml`. `release-plz release-pr`, the following checkout,
and changelog correction use `RELEASE_PLZ_TOKEN`, so the final PR head can
trigger its required checks. The checkout persists that credential for the
correction's `git push`; setting only the action environment is insufficient.

## Provision

Create a **fine-grained personal access token** on the `scbrown` account:

- Resource owner: `scbrown`.
- Repository access: **Only select repositories**, select **quipu**.
- Repository permissions: **Contents: Read and write**,
  **Pull requests: Read and write**. Metadata read access is automatic.
- No Actions, Workflows, Administration, or account permissions are needed
  for this PR-only credential. It does not change workflow files or bypass
  branch protection.
- Set an expiration and arrange renewal before it expires.

Store it as the repository **Actions secret `RELEASE_PLZ_TOKEN`** at
`scbrown/quipu` → Settings → Secrets and variables → Actions. Enter the value
there, never in a commit, issue, command argument, or log. A repository secret
is used because this job has no environment configured. A missing secret
fails explicitly before release-plz can create a release or PR; there is no
silent fallback to `GITHUB_TOKEN`.

This is the PAT implementation. An App is an alternative, not an additional
credential to provision: it would require Contents and Pull requests write
permissions, installation on this repository, an App ID/private key, and a
workflow step generating an installation token. That alternative is not wired
here. The PAT author is the owning account; Git commit author text alone does
not determine which credential authenticated the push.

See the [release-plz token contract](https://release-plz.dev/docs/github/token)
and [GitHub PAT guidance](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens).

## Why publication keeps its current credential

Changing the credential for the combined release action would also enable
`release: published` workflows. `crates.yml` still has that trigger, alongside
the inline publisher in `release.yml`; enabling both could race two publishers.
The explicit `release` and `release-pr` commands isolate PR authentication
without changing release-event delivery. The `release` step retains its ID and
`releases_created` output consumed by the artifact and publishing jobs.

The split does not fix changelog baseline selection. An earlier ordered split
was measured ineffective for that different problem: release-plz used the
published crate as its baseline, not tag existence. The inline changelog
corrector and both verification paths remain necessary.

A GitHub PAT cannot configure crates.io Trusted Publishing. Registry delivery
still requires the crate owner's publisher configuration for the existing
workflow and a successful publish verified at the registry. The GitHub
credential and registry configuration are separate provisioning steps even
when requested together.

## Activation and acceptance

Provision the secret before landing this staged change. Keep the required
checks and contributor approval policy intact. Existing parked release PRs
are handled by the existing approval automation; do not approve stale runs
or disable the automation to make an acceptance result look cleaner.

After the next normal main push creates or updates a release PR:

1. Record the generating Release run and release PR number. Read the PR's
   current `headRefOid` after changelog correction finishes.
2. Query branch protection for its current required contexts. At preparation
   time they are Format, Clippy (default), Clippy (shacl), Test (default),
   Test (shacl), and Build, with strict checks enabled.
3. Read workflow runs and check runs for that exact SHA. All required contexts
   must actually report and finish successfully; zero checks is failure.
   Re-read the PR SHA afterwards and repeat if it changed.
4. Confirm runs did not conclude `action_required` and did not require a
   manual or automated approval. Correlate their run IDs with the approval
   automation log; an unparked green run proves the workaround, not this fix.
   A reused bot-authored PR is not proof of new PAT-authored PR creation.
5. Verify the release operation still uses `GITHUB_TOKEN` and its existing
   inline delivery jobs. Treat later tag/assets, service deployment, and
   crates.io version movement as separate observations.

Read-only commands (substitute the observed PR and SHA):

```bash
gh pr view PR --repo scbrown/quipu --json author,headRefOid,url
gh api repos/scbrown/quipu/branches/main/protection/required_status_checks
gh api --paginate 'repos/scbrown/quipu/actions/runs?head_sha=SHA&per_page=100'
gh api --paginate 'repos/scbrown/quipu/commits/SHA/check-runs?per_page=100'
```

If activation fails, preserve the run IDs and errors. Revert the workflow
change through the normal reviewed route to restore the earlier credential
path; the existing approval automation remains the interim recovery. Do not
weaken required checks or contributor approval policy.
