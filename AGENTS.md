# quipu - Agent Instructions

## Project Overview

AI-native knowledge graph with strict ontology enforcement — Bobbin's knowledge module

## Conventions

- **Always use `just` instead of raw commands.** The justfile is configured with quiet output by default to save context — you only see errors and warnings.
- **Prefer subcommands over separate recipes.** Group related operations under a single recipe with a subcommand argument (e.g., `just docs build`, `just docs lint`) rather than creating separate top-level recipes (e.g., `just docs-build`, `just docs-lint`).
- **Keep source files small.** `scripts/check-file-size.sh` warns at 400 lines
  and fails at 500 (tests exempt). It is a **ratchet**: files already over the
  limit are grandfathered in `.file-size-baseline` at their current size and may
  shrink freely, but growing past that size fails. A new file over 500 fails
  outright. When a baselined file shrinks, run
  `scripts/check-file-size.sh --update-baseline` and commit — it never loosens
  an entry.

## Build Commands

```bash
just --list          # Show available commands
just setup           # Install pre-commit hooks
just check           # Run all quality checks
```

For verbose output when debugging:

```bash
just check verbose=true
```

## Documentation Commands

```bash
just docs build      # Build the book
just docs serve      # Serve locally with hot reload
just docs lint       # Lint markdown files
just docs check      # Full docs quality gate
```

## Quality Requirements

### Before Every Push

You MUST run and pass the full quality gate before pushing:

```bash
just check
```

This runs all pre-commit hooks including:

- Trailing whitespace and EOF checks
- YAML/JSON validation
- Merge conflict detection
- Markdown linting
**Do NOT push if any check fails.** Fix the issues and re-run.

### Test Requirements

- All existing tests must pass before pushing
- New functionality must include corresponding tests
- Tests are part of the `just check` quality gate

### Documentation Requirements

- User-facing changes MUST include documentation updates
- Run `just docs build` to verify the book builds cleanly
- Run `just docs check` to verify linting passes
- Update README.md if the change affects quick-start or usage

## Landing the Plane (Session Completion)

**When ending a work session**, complete ALL steps below. Work is NOT complete until `git push` succeeds.

1. **Run quality gates** — `just check` must pass
2. **Build docs** — `just docs build` must succeed (if docs changed)
3. **Commit and push**:

   ```bash
   git add <files>
   git commit -m "<type>: <description>"
   git push
   ```

4. **Verify** — All changes committed AND pushed

**CRITICAL RULES:**

- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing — that leaves work stranded locally
- NEVER say "ready to push when you are" — YOU must push
- If push fails, resolve and retry until it succeeds

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:970c3bf2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See <https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md> for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:

   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```

5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**

- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
