# Agent Testing Guide

This project controls a live browser session (Edge or Chrome). The testing approach must avoid touching real tabs the user cares about.

## Build and test commands

```bash
npm install          # Install deps + configure git hooks (via prepare script)
npm run build        # Bundle extension & build Rust binary
npm test             # Build + run all unit tests (no browser needed)
npm run test:unit    # Run TypeScript + Rust unit tests (no build, no browser)
npm run test:unit:ts # Run TypeScript unit tests only (fast, no build, no browser)
npm run test:integration  # Run integration tests (requires Chrome)
```

A **split hook gate** is active via `core.hooksPath=.githooks` (set by `npm install`):
- **pre-commit** (`.githooks/pre-commit`) runs fast unit checks (`npm run test:unit`).
- **pre-push** (`.githooks/pre-push`) runs heavier checks (`npm run rust:verify` and `npm run test:integration`) when Rust/build/hook-related files changed.
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
    helpers.ts       # Pure helper functions (version parsing, incognito state, event normalisation)
    lib/
      content.ts     # Content-script functions for execute-script primitive
      screenshot.ts  # Screenshot capture + OffscreenCanvas tiling
  tests/unit/    # TypeScript unit tests (no browser required)
    chrome-stub.ts      # Lightweight Chrome API stub
    helpers.test.ts     # Tests for pure extension helpers
    background.test.ts  # Behavioural tests (reconnect, sync, incognito)
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

**Version management:** All version files are synced by `scripts/bump-version.js` (exposed as `npm run bump:<kind>`). Never edit version fields manually.

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

- `scripts/bump-version.js` — syncs all version files (package.json, win32-x64, 3× Cargo.toml, lockfiles)
- `scripts/ci-wait-merge.sh` — waits for CI, merges PR, tags, creates GitHub release
- `scripts/test-mise-release.sh` — integration test comparing npm stable vs mise alpha channels
- `scripts/gen-version.js` — generates extension manifest version at build time

## Skills

The `skills/` directory contains agent skills installable via the Skills CLI (`npx skills add`):

- `skills/tabctl/` — CLI usage guide for agents
- `skills/release/` — Release automation (version bump → PR → merge → tag → release)
- `skills/git-commit/` — Conventional commit message generation

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
These tests validate extension helpers and Rust CLI/host logic. No browser needed.

Run:
- `npm run test:unit` (TypeScript + Rust unit tests, no build step)
- `npm run test:unit:ts` (TypeScript tests only, fastest feedback loop)
- `npm test` (builds first, then runs all unit tests)

Notes:
- TypeScript tests live in `src/tests/unit/` and run directly via Node 24's built-in
  `--experimental-strip-types` flag — no compilation step needed.
- `src/tests/unit/chrome-stub.ts` provides a lightweight Chrome API stub that records
  calls and returns predictable results.
- `src/tests/unit/helpers.test.ts` tests pure functions: version parsing, ID validation,
  incognito state tracking, and event normalisation.
- `src/tests/unit/background.test.ts` tests behavioural properties: reconnect guards,
  browser-state sync message shape, and incognito event tagging.
- Rust tests live in `rust/crates/*/src/` (unit) and `rust/crates/tabctl/tests/` (integration).
- Rust integration tests require Chrome and run via `npm run test:integration`.

## Required end-of-task checks
Always finish with:
1. `npm test` — all unit tests must pass
2. `npm run test:integration` — all integration tests must pass (if Chrome is available)
3. A minimal smoke test in a new window you create (safe URLs + unique `TEST-` prefix). Verify via `tabctl group-list` or `tabctl list`.
4. A screenshot-first smoke step: capture a screenshot before running selector-based extraction.
5. If multiple profiles are configured, verify the active profile with `tabctl profile-show` before running smoke tests.

> **Note:** Hooks provide split enforcement (fast on commit, heavy on push). If you bypass with `--no-verify` or `--no-verify` on push, run required checks manually.

Example (recommended for development):
```bash
ts=$(date +%s)
tabctl open --new-window --url https://example.com --url https://example.org --url https://example.net --group "TEST-Smoke-${ts}"
tabctl group-list --window last-focused
```


Screenshot-first example:
```bash
tabctl screenshot --tab <tabId> --mode viewport
tabctl inspect --tab <tabId> --signal selector --selector "link=a[href]" --selector-attr href-url
```

## Safe smoke tests (no mutations)
Run these anytime:
- `tabctl ping`
- `tabctl list`
- `tabctl analyze --stale-days 30`
- `tabctl inspect --tab <tabId> --signal page-meta --progress`
- `tabctl inspect --tab <tabId> --signal selector --selector "price=.price" --progress`
- `tabctl report --format json` (no `--out`)

## Controlled mutation tests (real Edge, minimal risk)
Only use a dedicated test window and clearly labeled groups.

### Setup a test window
1. Create a new Edge window.
2. Open three safe URLs: `https://example.com`, `https://example.org`, `https://example.net`.
3. Group the first two tabs and name the group `TEST-Tabctl-<timestamp>`.
4. Leave the third tab ungrouped.

### Archive test
1. Run `tabctl list` to find the test window id.
2. Archive only that window:
   `tabctl archive --window <windowId>`
3. Confirm in Edge:
   - A single Archive window exists.
   - A group named `W# - TEST-Tabctl-<timestamp>` exists.
   - An `W# - Ungrouped` group exists for the ungrouped tab.
4. Undo:
   `tabctl undo <txid>`

### Close test
1. Open a new tab in the test window (e.g. `https://example.com`).
2. Find its `tabId` via `tabctl list`.
3. Close that tab only:
   `tabctl close --tab <tabId> --confirm`
4. Undo:
   `tabctl undo <txid>`

### Report test
1. Run `tabctl report --window <windowId> --format md --out /tmp/tab-report.md`.
2. Verify the report includes descriptions for the example pages.

## Safe usage of analyze/close
`tabctl close` only works with explicit targets and is blocked for protected tabs by policy.

Recommended safe pattern:
1. Use `npm run test:integration` for automated close/undo testing in an isolated browser.
2. For manual testing, use a dedicated test profile or a brand-new Edge window with only test tabs.
3. Run `tabctl analyze`.
4. Use `tabctl close --tab <tabId> --confirm` for a single test tab.
5. Run `tabctl undo <txid>` to restore.

## Undo history sanity checks (synthetic)
You can validate undo with a single safe tab by inserting a synthetic record.

1. Append a single JSON line to `$XDG_STATE_HOME/tabctl/undo.jsonl` (or `~/.local/state/tabctl/undo.jsonl`) (manual or via a tiny script).
2. Use a unique `txid` and a safe URL (`https://example.com`).

Example line (single tab, forces a new window):

```
{"txid":"tx-test-undo-1","createdAt":1700000000000,"action":"close","summary":{"closedTabs":1},"undo":{"action":"close","tabs":[{"url":"https://example.com","title":"Example","pinned":false,"active":false,"from":{"windowId":0,"index":0,"groupId":-1,"groupTitle":null,"groupColor":null,"groupCollapsed":null}}]}}
```

Then run:
- `tabctl undo tx-test-undo-1`

This should open a single tab in a new window.

## Integration tests (isolated headless Chrome)
The integration test launches a headless Chrome with its own `--user-data-dir`, loads the extension via CDP, and runs real CLI commands against it. No real tabs are touched.

Run:
- `npm run test:integration`

This covers destructive paths (close, undo) safely. To test additional destructive commands (archive, dedupe), add Rust-side scenarios in `rust/crates/tabctl/tests/browser_integration.rs` (keep `scripts/ci/integration-bootstrap.js` as thin browser bootstrap only).
On Windows, the Rust browser integration test uses TCP transport (`TABCTL_TRANSPORT=tcp`) for CLI requests because named-pipe transport can intermittently stall in CI.

## Code architecture style

This codebase follows the **progressive disclosure architecture** pattern (see the `agentic-progressive-disclosure-architecture` skill). Top-level files are declarative (module declarations + re-exports), with implementation in deeper modules. Each subtree has its own `AGENTS.md` describing its scope and constraints. When adding new modules, repeat this pattern: API shape first, forwarding second, implementation deepest.

## Hard stop rules
- Never run `tabctl archive --all` or `tabctl close --apply` in a normal profile.
- Never run `tabctl close` without explicit `--tab`, `--group`, or `--window` targets.
- Always verify window ids and group names before any mutation.
