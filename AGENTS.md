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
- **pre-commit** (`.githooks/pre-commit`) builds/bundles the extension and runs its pure byte-budget tests plus Rust unit checks (`npm run test:unit`).
- **pre-push** (`.githooks/pre-push`) runs heavier checks (`npm run rust:verify` and `npm run test:integration`) when Rust/build/hook-related files changed.
- **local cross-target checks** are opt-in via `npm run check:targets` / `make dev-check-targets`; they check Apple Silicon and Intel macOS and require both rustup targets plus Xcode command-line tools.
- Product support is macOS only, Edge and Chrome, with Unix-domain sockets. All CI and release jobs run on macOS.

## Project architecture

The single `tabctl` binary (Rust) serves as both the CLI and the native messaging host. The `tabctl host` subcommand is the native messaging entry point, invoked by the browser automatically.

```
rust/
  crates/
    tabctl/      # Single binary: CLI + host entry point
    host/        # Native messaging host logic + command orchestration
    shared/      # Shared utilities (config, profiles, protocol)
src/
  extension/     # Chrome extension (background service worker) — only TypeScript component
    lib/
      content.ts     # Content-script functions for execute-script primitive
      screenshot.ts  # Screenshot capture + OffscreenCanvas tiling
  tests/unit/    # Unit tests (no browser required)
```

**Architecture:** The extension is a thin primitive layer (~16 Chrome API wrappers with `p:` prefix). All command orchestration lives in the Rust host (`rust/crates/host/src/host_impl/orchestrate/`), which sequences primitives per CLI request. This makes orchestration logic unit-testable without a browser.

**Data flow:** CLI → Unix-domain socket → Host (`tabctl host`) → orchestration → primitive sequence → Native messaging → Extension → Chrome APIs

## CLI Usage Rules for Agents

### Scope-First Rule
Choose and verify the scope **before** executing a browser query or mutation. Browser operations use `tabctl query`; scope is expressed as GraphQL field arguments, not legacy CLI flags.

**Required scoping pattern:**
```bash
# Read only the intended window, group, or tab
tabctl query '{ tabs(windowId: 123) { total hasMore items { tabId title url pinned groupId } } }'
tabctl query '{ tabs(windowId: 123, groupTitle: "TEST-Work") { items { tabId title url } } }'
tabctl query '{ tab(id: 456) { tabId windowId title url pinned groupId } }'

# Preview an explicitly selected, test-created tab in an isolated test profile
tabctl --profile test-profile query 'mutation { closeTabs(tabIds: [456], dryRun: true) { txid dryRun plannedTabs skippedTabs tabs { tabId title url } skipped { tabId reason } } }'
```

Example IDs and `test-profile` are placeholders. Mutation examples must target only tabs created by the test, never normal user tabs.

**Scope arguments (prefer the narrowest supported scope):**
1. A specific tab: `tab(id: ...)` for reads; `tabIds: [...]` for mutations that accept them.
2. A group: `groupId` or `groupTitle`, constrained by `windowId` where supported.
3. A window: `windowId`.
4. An intentionally unfiltered inventory read, such as `windows`; never an unscoped mutation.

Arguments differ by field. In particular, `closeTabs` requires explicit `tabIds`; it does not accept window/group selectors. Inspect with a scoped read first, then pass the verified IDs. Use `tabctl schema` to check supported arguments.

### Required Scope Usage
Always scope reads such as `tabs`, `analyze`, `inspectTabs`, `readTabs`, and `reportTabs` to the intended window/group/tabs where supported. Select IDs explicitly for destructive mutations. There is no `--all` flag for GraphQL queries; an unfiltered read must be deliberate.

### Preview and Confirmation Rules
`closeTabs` and `deduplicateTabs` preview when `confirm` is omitted or false. `closeTabs(dryRun: true)` remains a preview even with `confirm: true`. Close previews return `dryRun: true`, no transaction ID, and planned/skipped targets; review the IDs and exclusion reasons before executing.

```bash
# After reviewing the preview, close only those test-created IDs
tabctl --profile test-profile query 'mutation { closeTabs(tabIds: [456], confirm: true) { txid dryRun closedTabs skippedTabs skipped { tabId reason } } }'

# Dedupe only a test-created window: preview first, then confirm
tabctl --profile test-profile query 'mutation { deduplicateTabs(windowId: 123) { txid closedTabs candidateTabs { tabId title url } } }'
tabctl --profile test-profile query 'mutation { deduplicateTabs(windowId: 123, confirm: true) { txid closedTabs } }'
```

`archiveTabs` executes immediately and has no `confirm` or `dryRun` argument. Inspect the selected test-created tabs first, then archive explicit IDs:

```bash
tabctl --profile test-profile query 'mutation { archiveTabs(tabIds: [456, 789]) { txid archivedTabs } }'
```

### Command Workflow
1. **Read first** — Query scoped tabs/groups and follow pagination (`hasMore`/`offset`) when needed.
2. **Verify IDs and ownership** — Confirm the selected profile, window/group/tab IDs, and that every mutation target was created by the test.
3. **Preview where supported** — Review `closeTabs` planned/skipped targets or `deduplicateTabs` candidates.
4. **Execute with explicit scope** — Confirm close/dedupe only after review; retain the returned `txid` for recovery.
5. **Check the result** — Use fresh scoped GraphQL reads and `tabctl history`. A nonzero exit or GraphQL `errors` means failure, not success.
6. **Undo by transaction ID** — Prefer the exact transaction over `latest`, which is safe only in a fully isolated test profile:

```bash
tabctl --profile test-profile query 'mutation { undoAction(txid: "tx-from-mutation") { txid summary } }'
```

Undo restores window placement, grouping, and ordering. Do not blindly replay an already-undone or uncertain transaction; stop and inspect its history/recovery error.

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
5. The `release.yml` workflow publishes macOS Intel/Apple Silicon binaries and extension assets to GitHub Releases, and the root `tabctl` npm package with a universal macOS native executable and bundled extension. Both mise/GitHub and npm installation remain supported.

**Merge strategy:** Always use normal merge for release PRs (not squash) to preserve commit identity.

## Scripts

- `scripts/bump-version.js` — bumps `rust/Cargo.toml`, mirrors package versions, and refreshes lockfiles
- `scripts/check-targets.sh` — optional cargo check of both macOS architectures
- `scripts/ci-wait-merge.sh` — waits for CI, merges PR, tags, creates GitHub release
- `scripts/gen-version.js` — generates extension manifest version at build time
- `scripts/package-npm.js` — validates arm64/x86_64 release inputs and creates/verifies `dist/npm/tabctl` with `lipo`; no optional platform packages or postinstall download
- `scripts/verify-npm-package.js` — verifies the actual tarball, offline npm bin link, version, and sandboxed setup without launching a browser

## Skills

The `skills/` directory contains agent skills installable via the Skills CLI (`npx skills add`):

- `skills/tabctl/` — CLI usage guide for agents
- `skills/release/` — Release automation (version bump → PR → merge → tag → release)
- `skills/git-commit/` — Conventional commit message generation
- `.github/skills/smoke-test/` — End-of-task smoke test: unit tests, integration tests, and live browser mutation + undo verification in a disposable `TEST-Smoke-*` window

## Principles (read first)
- Only mutate tabs that the test itself created.
- Never run unscoped destructive GraphQL mutations or mutate normal user tabs.
- Use a unique, recognizable prefix for test groups and windows, e.g. `TEST-Tabctl-<timestamp>`.
- Prefer scoped GraphQL `tabs`, `analyze`, and `reportTabs` reads for smoke tests; use `closeTabs` and `archiveTabs` only on test-created tabs in a controlled test window.
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
tabctl --profile chrome-work query '{ tabs(windowId: 123) { total items { tabId title url } } }'
```

When creating smoke tests, ensure you are connected to the correct profile. Use `tabctl profile-list` to see all available profiles.

## Unit tests (no browser required)
These tests validate CLI/host helpers and the extension's pure native-message byte-budget boundary. No browser needed.

Run:
- `npm test` (builds/bundles first, then runs extension/packaging tests and Rust verification)
- `npm run test:unit` (extension build and pure extension/packaging tests plus Rust tests; pre-commit gate)
- `npm run test:extension` (compiled native-message tests only; requires a prior extension build)
- `npm run test:packaging` (real tiny clang/lipo executables; no browser, Rust release build, or publishing)

Notes:
- Extension byte-budget tests are in `src/extension/lib/native-message.test.ts`, compiled to `dist/extension/lib/native-message.test.js`, and run with Node's built-in test runner.
- Browser behavior belongs in the isolated real-browser integration harness.
- For type checks without a build, use `tsc -p tsconfig.json --noEmit`. Never leave `dist` with an unbundled background script: use `npm run build:extension` or `npm run build` before browser tests.

## Required end-of-task checks

Always finish by running the `/smoke-test` skill (`.github/skills/smoke-test/SKILL.md`). The skill must use the automated runner (`npm run test:smoke`) instead of manual smoke commands. It covers:
1. `npm test` — unit tests
2. `npm run test:integration` — integration tests (if Chrome is available)
3. Isolated smoke profile verification
4. Read-only live browser checks (`ping` and scoped GraphQL `tabs`, `analyze`, `reportTabs`)
5. Mutation round-trips (close + undo, archive + undo) in a disposable `TEST-Smoke-<timestamp>` window
6. Automated cleanup of smoke-created tabs/windows and browser teardown

> **Note:** Hooks provide split enforcement (fast on commit, heavy on push). If you bypass with `--no-verify` on push, run `npm test` and `npm run test:integration` manually.

## Smoke tests and integration tests

See `.github/skills/smoke-test/SKILL.md` for the full automated procedure — safe read-only checks, controlled mutation tests (close + undo, archive + undo) in a disposable `TEST-Smoke-*` window, and synthetic undo sanity checks. Do not run ad hoc live-browser smoke mutations outside the automated runner unless debugging a specific runner failure.

Integration tests run against an isolated headless Chrome (`npm run test:integration`) and cover destructive paths safely. To test additional destructive commands (archive, dedupe), add Rust-side scenarios in `rust/crates/tabctl/tests/browser_integration.rs` (keep `scripts/ci/integration-bootstrap.js` as thin browser bootstrap only). Use the production macOS Unix-domain socket transport.

## Code architecture style

This codebase follows the **progressive disclosure architecture** pattern (see the `agentic-progressive-disclosure-architecture` skill). Top-level files are declarative (module declarations + re-exports), with implementation in deeper modules. Each subtree has its own `AGENTS.md` describing its scope and constraints. When adding new modules, repeat this pattern: API shape first, forwarding second, implementation deepest.

## Hard stop rules
- Never run unscoped `archiveTabs`/`deduplicateTabs` or mutate normal user tabs.
- Never run `closeTabs` without explicit, verified `tabIds`; always preview before confirming.
- Always verify the profile, window/group/tab IDs, and test ownership before any mutation.
