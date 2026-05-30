# Agent Testing Guide

This project controls a live browser session (Edge or Chrome). The testing approach must avoid touching real tabs the user cares about.

## Build and test commands

```bash
npm install          # Install deps + configure git hooks (via prepare script)
npm run build        # Bundle extension & build Rust binary
npm test             # Build + run unit tests (no browser needed)
npm run test:integration  # Run integration tests (requires Chrome)
```

A **split hook gate** is active via `core.hooksPath=.githooks` (set by `npm install`):
- **pre-commit** (`.githooks/pre-commit`) runs fast unit checks (`npm run test:unit`).
- **pre-push** (`.githooks/pre-push`) runs heavier checks (`npm run rust:verify` and `npm run test:integration`) when Rust/build/hook-related files changed.
- **local cross-target checks** are opt-in via `npm run check:targets` / `make dev-check-targets`; they are not mandatory in pre-push because this workspace's SQLite dependency chain compiles C code during cross-target `cargo check`.
- CI `wsl` job validates the WSL -> Windows invocation bridge (setup + Windows command/native-host invocation + integration handoff); it does not compile Rust inside WSL.

## Project architecture

The single `tabctl` binary (Rust) serves as both the CLI and the native messaging host. The `tabctl host` subcommand is the native messaging entry point, invoked by the browser automatically.

```
rust/
  crates/
    tabctl/      # Single binary: CLI + host entry point
    host/        # Native messaging host logic + command orchestration
    shared/      # Shared utilities (config, profiles, WSL support)
src/
  extension/     # Chrome extension (background service worker) — only TypeScript component
    lib/
      content.ts     # Content-script functions for execute-script primitive
      screenshot.ts  # Screenshot capture + OffscreenCanvas tiling
  tests/unit/    # Unit tests (no browser required)
```

**Architecture:** The extension is a thin primitive layer (~16 Chrome API wrappers with `p:` prefix). All command orchestration lives in the Rust host (`rust/crates/host/src/host_impl/orchestrate/`), which sequences primitives per CLI request. This makes orchestration logic unit-testable without a browser.

**Data flow:** CLI → Unix socket/named pipe → Host (`tabctl host`) → orchestration → primitive sequence → Native messaging → Extension → Chrome APIs

## CLI Usage Rules for Agents

### Scope-First Rule
Always specify scope options **before** running any query or mutation command. This ensures predictable results and avoids accidental broad operations.

**Required scoping pattern:**
```bash
# Good: Explicit scope
tabctl list --window 123
tabctl list --group "Work"
tabctl list --tab 456 --tab 789
tabctl close --tab 456 --confirm

# Bad: No scope (defaults to all, risky for mutations)
tabctl close --confirm  # NEVER do this
```

**Scope options (in order of specificity):**
1. `--tab <id>` - Most specific, target individual tabs
2. `--group <name>` or `--group-id <id>` - Target a group
3. `--window <id>` - Target a window
4. `--all` - Explicit "all" (only for read operations)

### Required Scope Usage
Always include an explicit scope option when running commands that accept scope (list, analyze, dedupe, inspect, report, close, archive, group-list). Use `--all` when you truly intend to target everything.

### Confirmation Rule for Destructive Commands
Destructive commands (`close`, `archive`, `dedupe --confirm`) require explicit confirmation AND explicit scope:

```bash
# Pattern: scope first, then --confirm
tabctl close --tab 456 --confirm
tabctl close --group "Temp" --window 123 --confirm
tabctl archive --window 123

# For dedupe, always preview first:
tabctl dedupe --window 123           # Preview plan
tabctl dedupe --window 123 --confirm # Execute after review
```

### Command Workflow
1. **List first** - Use `tabctl list` with scope to see what will be affected
2. **Verify IDs** - Confirm window/group/tab IDs before mutations
3. **Execute with scope** - Run mutation with explicit `--tab`, `--group`, or `--window`
4. **Check result** - Verify with `tabctl list` or `tabctl history`
5. **Undo if needed** - Use `tabctl undo --latest` or `tabctl undo <txid>`

## Commit message style
- Use Conventional Commits (`type(scope): subject`), with scope optional.
- Keep the subject in present-tense, lowercase imperative form.
- Do not add a trailing period.

## Release workflow

Direct pushes to `main` are blocked by a branch ruleset (requires PR + CI checks + Copilot review).

**Version management:** `rust/Cargo.toml` is the Rust version source of truth. `scripts/bump-version.js` (exposed as `npm run bump:<kind>`) updates that workspace version and mirrors it to the npm/package manifests. Never edit version fields manually.

```bash
npm run bump:alpha    # next alpha
npm run bump:rc       # alpha → rc.1, or rc.N+1
npm run bump:stable   # strip pre-release suffix
npm run bump:patch    # patch bump
npm run bump:minor    # minor bump
npm run bump:major    # major bump
```

**Release flow:** See `skills/release/SKILL.md` for the full process. Summary:
1. `npm run bump:alpha` (or other kind) on a `chore/release-v{NEW}` branch
2. `npm test` + `npm run build`
3. Commit, push, open PR
4. `scripts/ci-wait-merge.sh <PR#> --tag v{NEW}` — waits for CI, merges (normal merge, not squash), tags, creates GitHub release
5. The `release.yml` workflow builds binaries and publishes to npm

**Merge strategy:** Always use normal merge for release PRs (not squash) to preserve commit identity.

## Scripts

- `scripts/bump-version.js` — bumps `rust/Cargo.toml`, mirrors package versions, and refreshes lockfiles
- `scripts/check-targets.sh` — optional local cross-target cargo check; requires extra host toolchains for Linux/Windows targets
- `scripts/ci-wait-merge.sh` — waits for CI, merges PR, tags, creates GitHub release
- `scripts/test-mise-release.sh` — integration test comparing npm stable vs mise alpha channels
- `scripts/gen-version.js` — generates extension manifest version at build time

## Skills

The `skills/` directory contains agent skills installable via the Skills CLI (`npx skills add`):

- `skills/tabctl/` — CLI usage guide for agents
- `skills/release/` — Release automation (version bump → PR → merge → tag → release)
- `skills/git-commit/` — Conventional commit message generation
- `.github/skills/smoke-test/` — End-of-task smoke test: unit tests, integration tests, and live browser mutation + undo verification in a disposable `TEST-Smoke-*` window

## Principles (read first)
- Only mutate tabs that the test itself created.
- Never run `archive --all` or `close --apply` in a normal browsing session.
- Use a unique, recognizable prefix for test groups and windows, e.g. `TEST-Tabctl-<timestamp>`.
- Prefer `list`, `analyze`, and `report` for smoke tests; use `close` and `archive` only in a controlled test window.
- Always add or update tests for new features.
- Always end work by running unit tests and a minimal smoke test in a new window you create (see Required end-of-task checks).

## Undo is critical
- Treat undo as a first-class safety feature for every mutating action.
- Any new mutating command must record a complete undo payload and include tests.
- Undo should restore window placement, group metadata, and tab ordering whenever possible.

## Preconditions
- Edge is open.
- The extension is loaded (`extension/`) and connected to the native host.
- The native host manifest is installed (use `tabctl setup --browser edge --extension-id <id>`). Setup writes the wrapper script, native messaging manifest, and registers the profile.
- For development, use `cargo run -p tabctl --` or a debug build so a stable global `tabctl` can stay installed.

## Profile awareness
When multiple profiles are configured, verify which browser the CLI is targeting before running commands:

```bash
tabctl profile-show --json
```

Check the `name` and `browser` fields in the output. To target a specific browser for a single command, use `--profile <name>`:

```bash
tabctl list --profile chrome-work --all
```

When creating smoke tests, ensure you are connected to the correct profile. Use `tabctl profile-list` to see all available profiles.

## Unit tests (no browser required)
These tests validate the CLI/host helpers using a mocked socket and extension logic using a chrome API stub. No browser needed.

Run:
- `npm test` (builds first, then runs all unit tests)

Notes:
- Source in `src/tests/unit/`, compiled to `dist/tests/unit/`.
- CLI tests use a mock socket to avoid browser interaction.
- Extension tests (e.g., `extension.tabs.test.ts`) use a lightweight chrome stub on `globalThis.chrome` that records API calls and returns predictable results.

## Required end-of-task checks

Always finish by running the `/smoke-test` skill (`.github/skills/smoke-test/SKILL.md`). It covers:
1. `npm test` — unit tests
2. `npm run test:integration` — integration tests (if Chrome is available)
3. Profile verification
4. Read-only live browser checks (`ping`, `list`, `analyze`, `report`)
5. Mutation round-trips (close + undo, archive + undo) in a disposable `TEST-Smoke-<timestamp>` window
6. Clean up of the test window

> **Note:** Hooks provide split enforcement (fast on commit, heavy on push). If you bypass with `--no-verify` on push, run `npm test` and `npm run test:integration` manually.

## Smoke tests and integration tests

See `.github/skills/smoke-test/SKILL.md` for the full procedure — safe read-only checks, controlled mutation tests (close + undo, archive + undo) in a disposable `TEST-Smoke-*` window, and synthetic undo sanity checks.

Integration tests run against an isolated headless Chrome (`npm run test:integration`) and cover destructive paths safely. To test additional destructive commands (archive, dedupe), add Rust-side scenarios in `rust/crates/tabctl/tests/browser_integration.rs` (keep `scripts/ci/integration-bootstrap.js` as thin browser bootstrap only). On Windows, use `TABCTL_TRANSPORT=tcp`.

## Code architecture style

This codebase follows the **progressive disclosure architecture** pattern (see the `agentic-progressive-disclosure-architecture` skill). Top-level files are declarative (module declarations + re-exports), with implementation in deeper modules. Each subtree has its own `AGENTS.md` describing its scope and constraints. When adding new modules, repeat this pattern: API shape first, forwarding second, implementation deepest.

## Hard stop rules
- Never run `tabctl archive --all` or `tabctl close --apply` in a normal profile.
- Never run `tabctl close` without explicit `--tab`, `--group`, or `--window` targets.
- Always verify window ids and group names before any mutation.
