# CI/CD and Releases

Quipu uses GitHub Actions for continuous integration and release-plz for
automated versioning and changelog generation.

## CI Pipeline

Every push to `main` and every pull request triggers the CI workflow
(`.github/workflows/ci.yml`). Jobs run in parallel:

| Job | What it does |
|-----|-------------|
| **fmt** | `cargo fmt --check` |
| **clippy** | Linting with multiple feature combinations (default, SHACL) |
| **test** | Test suite across feature matrix |
| **build** | Full compilation check |
| **check** | Pre-commit hooks on all files |
| **lint-markdown** | markdownlint-cli2 on documentation |

All jobs use cargo caching for fast iteration.

## Release Automation

Pushes to `main` trigger the release workflow
(`.github/workflows/release.yml`), which uses
[release-plz](https://release-plz.ieni.dev/) to:

1. Analyze conventional commits since the last release
2. Bump the version in `Cargo.toml`
3. Generate a changelog from commit messages
4. Create a GitHub release with a git tag

### Conventional Commits

Commit messages drive changelog categories:

| Prefix | Category |
|--------|----------|
| `feat:` | Added |
| `fix:` | Fixed |
| `refactor:` | Changed |
| `doc:` | Documentation |
| `test:` | Testing |
| `chore:` | Miscellaneous |
| `ci:` | CI/CD |

Commits with `security` in the body get a **Security** category.

### Configuration

- **release-plz.toml** -- enables git releases and tags, disables crates.io
  publishing
- **cliff.toml** -- [git-cliff](https://git-cliff.org/) template for
  changelog formatting with GitHub links and commit SHAs

## Documentation Deployment

The docs workflow (`.github/workflows/docs.yml`) builds the mdbook and
deploys to GitHub Pages on pushes to `main`:

1. Build: `mdbook build docs/book`
2. Deploy: upload to GitHub Pages (main branch only)

## Pre-commit Hooks

Local development uses pre-commit hooks (`.pre-commit-config.yaml`):

- Trailing whitespace and end-of-file fixes
- YAML and JSON validation
- Merge conflict detection
- Large file checks
- Markdown linting (markdownlint-cli2)
- File size limits (warn at 400 lines, error at 500 for Rust source)

Install hooks with:

```bash
just setup
```

## Quality Gate

Before pushing, run the full quality gate:

```bash
just check       # All pre-commit hooks
just docs check  # Markdown lint + mdbook build
```
