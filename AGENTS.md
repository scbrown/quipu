# quipu - Agent Instructions

## Project Overview

AI-native knowledge graph with strict ontology enforcement — Bobbin's knowledge module

## Conventions

- **Always use `just` instead of raw commands.** The justfile is configured with quiet output by default to save context — you only see errors and warnings.
- **Prefer subcommands over separate recipes.** Group related operations under a single recipe with a subcommand argument (e.g., `just docs build`, `just docs lint`) rather than creating separate top-level recipes (e.g., `just docs-build`, `just docs-lint`).

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
