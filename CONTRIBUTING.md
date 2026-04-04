# Contributing to quipu

## Using Just

This project uses [just](https://github.com/casey/just) as a command runner. **Always prefer `just` commands over raw tool commands** — they're configured with sensible defaults.

```bash
just --list          # Show available commands
just setup           # Install pre-commit hooks
just check           # Run all quality checks
```

## Setup

1. Install [just](https://github.com/casey/just)
2. Install [pre-commit](https://pre-commit.com/)
3. Run `just setup` to install git hooks

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
just check           # Run all quality checks
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
