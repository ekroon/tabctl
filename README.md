# tabctl

Every open tab is a thread you forgot to pull. Tabctl helps you query and change them safely.

A command-line instrument for browser tab orchestration, now centered on a GraphQL API exposed through `tabctl query` and `tabctl schema`, plus `ping` and `history` convenience commands. Built for humans who hoard tabs and the AI agents who clean up after them.

## Install

```bash
mise use -g github:ekroon/tabctl   # install the tabctl binary
tabctl setup --browser edge --extension-id <id>   # or: --browser chrome --extension-id <id>
# Load the extension: edge://extensions → Developer mode → Load unpacked → paste: ~/.local/state/tabctl/extension/
tabctl ping
```

Setup writes the wrapper script, native messaging manifest, and registers the profile in one step. Works on macOS, Linux, and Windows. If it pings back, the wire is live. You're connected.

### Alternative: build from source

```bash
cargo install --path rust/crates/tabctl
```

> **Legacy:** `npm install -g tabctl` still works for the Node.js-based distribution but is no longer the primary install method. No Node.js or Go is required at runtime — the single `tabctl` binary handles everything.

## Agent Skill

Give your coding agent eyes into the browser. Install the tabctl skill via the Skills CLI:

```bash
npx skills add https://github.com/ekroon/tabctl --skill tabctl -a opencode -a github-copilot -a claude-code
```

## Safety

Nothing leaves your machine. No cloud. No telemetry. Just a socket between your terminal and your browser, quiet as rain on neon.

Every mutation is undoable — `undoAction` rewinds closes, archives, and group changes like they never happened. A configurable policy layer shields pinned tabs and protected domains from accidental destruction. You pull the trigger; tabctl keeps the safety on until you mean it.

## What You Can Say

When tabctl is installed as a skill, your agent sees what you see. Just talk to it.

> *"Which of my tabs can I close?"*
> The agent scans for duplicates, stale pages, and tabs you haven't touched in days — then offers to clean house.

> *"Are any of my open tabs relevant to my note on Project Helios?"*
> When connected to Obsidian, your agent cross-references every open tab against your notes and surfaces the ones that matter.

> *"I just finished researching service mesh architectures. Organize what I found."*
> Groups your tabs by theme, extracts key URLs, and drops a summary into your notes — before you forget what you were looking at.

> *"Where's that AWS pricing page I had open somewhere?"*
> The agent searches your open tabs and groups by title and URL — and brings it back into focus.

> *"Pull every error message from my open Sentry tabs into a markdown table."*
> The agent reads each tab, extracts what you need, and formats it — no copy-paste, no context switching.

> *"Group everything by project. You know which ones."*
> Your agent infers context from URLs, titles, and your workspace — then sorts ninety tabs into five groups with names that actually make sense.

---

`tabctl` is a single Rust binary that serves as both the CLI and the native messaging host. The CLI sends commands over a Unix socket (or named pipe on Windows) to the host, which proxies them to the browser extension via native messaging. The `tabctl host` subcommand is the native messaging entry point — invoked automatically by the browser, not manually.

This repo contains:
- Chrome/Edge extension (`src/extension/`, the only TypeScript component)
- Rust workspace (`rust/crates/*`) — single `tabctl` binary for CLI + host + shared runtime
- Node packaging/build scripts for distribution (legacy)

## Quick Start

### 1. Build and install

```bash
cargo install --path rust/crates/tabctl   # puts tabctl on your PATH
```

For development with the full build pipeline (extension + Rust):

```bash
npm install
npm run build
```

Local development shortcuts are available via `Makefile`:

```bash
make dev-up BROWSER=edge PROFILE=edge
make dev-run PROFILE=edge CMD="list --all --json"
make dev-run-release-like PROFILE=edge CMD="list --all --json"
```

If `npm` is not on PATH in your shell, override it per command:
```bash
make dev-build NPM=~/.local/share/mise/shims/npm
```

### 2. Set up your browser

Run setup to write the manifest, wrapper script, and profile registration:

<!-- test: "setup explicit --extension-id overrides auto-derived ID" -->
```bash
tabctl setup --browser chrome
```

This will:
1. Write the native messaging manifest and wrapper script
2. Register the browser profile in `profiles.json`
3. Download the version-pinned release extension asset (`tabctl-extension.zip` + `.sha256`) into the tabctl data directory
4. Sync the managed unpacked extension directory to the tabctl version (`~/.local/state/tabctl/extension/`)
5. Derive the extension ID from the managed extension path (or use explicit `--extension-id`)
6. Print the path for loading as an unpacked extension in `chrome://extensions`

For local dev builds (no GitHub download), point setup at an unpacked directory:
```bash
tabctl setup --browser chrome --extension-dir dist/extension
```

> **Edge?** Use `--browser edge` and load from `edge://extensions` instead.
>
> **Cross-platform:** setup works on macOS, Linux, and Windows. On Windows, setup verifies connectivity after writing setup artifacts and checks the runtime extension ID reported by the browser. Connectivity failures and runtime extension ID mismatches exit non-zero and print manual recovery steps (including expected vs runtime IDs).

Optional setup release overrides:
- Flags: `--extension-dir`, `--release-repo`, `--release-tag` (or `--release-version`), `--release-asset`, `--skip-extension-download`
- Env vars: `TABCTL_SETUP_EXTENSION_DIR`, `TABCTL_RELEASE_REPO`, `TABCTL_RELEASE_TAG`, `TABCTL_RELEASE_ASSET`, `TABCTL_SETUP_FETCH_EXTENSION=0`
- Precedence: flags override env vars, then built-in defaults; if download fails, setup continues and includes warning details in setup output.

### 3. Verify and explore

```bash
tabctl ping
tabctl query '{ tabs { total items { tabId title url } } }'
tabctl schema
```

> **Multiple browsers?** See [Multi-Browser Setup](#multi-browser-setup) for running tabctl with both Chrome and Edge.

## Commands

| Command | Description |
|---------|-------------|
| `tabctl query '<GRAPHQL>'` | Query and mutate browser state through GraphQL |
| `tabctl schema` | Print the GraphQL schema |
| `tabctl ping` | Check host/browser connectivity and runtime version sync |
| `tabctl history` | Show recent undo history entries |
| `tabctl setup`, `doctor`, `policy`, `profile-*` | Local/admin profile management |

Read-only browser features include:
- `inspectTabs` for page metadata and selector-based reads
- `readTabs` for page HTML → Markdown conversion with Kreuzberg preprocessing and per-tab diagnostics
- `reportTabs`, `captureScreenshots`, and browser-state history queries for summaries and context

See [CLI.md](CLI.md) for the full command reference, options, and examples.

## GraphQL examples

```bash
# Query tabs and groups
tabctl query '{ windows { windowId groups { groupId title } tabs { tabId title url groupTitle } } }'

# Analyze stale and duplicate tabs
tabctl query '{ analyze(windowId: 123, staleDays: 30) { totalTabs duplicateTabs staleTabs } }'

# Inspect page metadata
tabctl query 'query { inspectTabs(tabIds: [456], signals: ["page-meta"]) { entries { tabId signals { name valueJson } } } }'

# Inspect selectors with typed attrs and filters
tabctl query 'query { inspectTabs(windowId: 123, selectors: [{ name: "prices", selector: ".price", attr: "text", all: true }, { name: "buy_now_visible", selector: "button.buy-now", attr: "visible" }, { name: "buy_now_style", selector: "button.buy-now", attr: "styles", styleProps: ["color", "background-color"] }, { name: "review_count", selector: ".review", attr: "count" }, { name: "email_value", selector: "input[type=email]", attr: "value" }]) { entries { tabId url signals { name valueJson } } } }'

# Read tab content as Markdown (Kreuzberg preprocessing by default)
tabctl query 'query { readTabs(windowId: 123, extract: true, maxChars: 30000) { entries { tabId title url markdown chars truncated extracted cached status emptyReason diagnostics { source sourceHtmlChars sourceTextChars documentReadyState truncatedHtml cachedAt cacheAgeMs } error } } }'

# Generate reports
tabctl query '{ reportTabs(windowId: 123) { entries { tabId title url description } } }'

# Capture screenshots
tabctl query 'query { captureScreenshots(tabIds: [456], mode: "viewport") { entries { tabId tiles { index width height } } } }'

# Inspect persisted browser-state history for future restore tooling
tabctl query 'query { latestBrowserState { snapshotId reason groups { logicalGroupId title browserGroupId tabUrls } } }'
tabctl query 'query { browserStateHistory(limit: 10) { snapshotId recordedAt reason eventCount eventKinds } }'
tabctl query 'query { browserStateGroupHistory(title: "Research", limit: 10) { snapshotId logicalGroupId title browserGroupId tabUrls } }'

# Open tabs in a new grouped window
tabctl query 'mutation { openTabs(urls: ["https://example.com"], group: "Research", newWindow: true) { windowId groupId tabs { tabId url } } }'

# Close tabs with undo support
tabctl query 'mutation { closeTabs(tabIds: [456], confirm: true) { txid closedTabs } }'
tabctl query 'mutation { undoAction(latest: true) { txid summary } }'
```

## Agent skills

Install the tabctl skill for agents (OpenCode, Claude Code, Codex, etc.) via the Skills CLI:

```bash
npx skills add https://github.com/ekroon/tabctl --skill tabctl -a opencode
```

Install globally:

```bash
npx skills add https://github.com/ekroon/tabctl --skill tabctl --global -a opencode
```

## Policy (protect tabs)
By default the CLI loads a policy file from:
`<configDir>/policy.json` (default: `~/.config/tabctl/policy.json`)

Set `TABCTL_CONFIG_DIR` to override the config directory.

This is a **protection-only** policy that marks tabs as ineligible for agent actions.
Example:

```json
{
  "protect": {
    "pinned": true,
    "groupTitles": ["🔒"]
  }
}
```

Create a default policy file:

<!-- test: "policy init creates default file" -->
```bash
tabctl policy --init
```

`tabctl setup` does not install a default policy.
See `config/policy.example.json` for a starter template.

## Configuration
Config directory: `TABCTL_CONFIG_DIR` → `$XDG_CONFIG_HOME/tabctl` → `~/.config/tabctl`

An optional `config.json` in the config directory can set `dataDir` to override where state files (socket, undo log) are stored. When `TABCTL_CONFIG_DIR` is set but no `dataDir` is configured, data defaults to `<configDir>/data/`; otherwise it uses `$XDG_STATE_HOME/tabctl` (or `~/.local/state/tabctl`).

See [CLI.md](CLI.md#configuration) for full details.

## Runtime state
- Socket: `<dataDir>/tabctl.sock` (default: `~/.local/state/tabctl/tabctl.sock`)
- Undo log: `<dataDir>/undo.jsonl` (default: `~/.local/state/tabctl/undo.jsonl`)
- Browser-state history DB: `<dataDir>/state.db` (default: `~/.local/state/tabctl/state.db`)
- Profile registry: `<configDir>/profiles.json`
- Windows pipe endpoint file: `<dataDir>/pipe-endpoint`

## Windows + WSL transport

On Windows, the host exposes a named-pipe endpoint model:
- Windows native clients use a named pipe endpoint (`\\.\pipe\tabctl-<hash>`).
- WSL/Linux clients use a Windows named-pipe bridge; the Windows host publishes `<dataDir>/pipe-endpoint`, and the WSL CLI relays through `powershell.exe`.

WSL endpoint discovery (CLI):
1. `TABCTL_SOCKET` (explicit endpoint).
2. `pipe-endpoint` file discovery from resolved data dir (and equivalent `/mnt/c/Users/*/.../tabctl/.../pipe-endpoint` locations).

WSL named-pipe mode:
- This is the default WSL transport now.
- The CLI discovers the pipe endpoint from `<dataDir>/pipe-endpoint` (including the mirrored `/mnt/c/Users/*/...` candidate paths used for other WSL bridge files).
- TCP is disabled for the WSL transport path.

Relevant knobs: `TABCTL_SOCKET`, `TABCTL_PROFILE`, `TABCTL_DATA_DIR`, `TABCTL_STATE_DIR`, `TABCTL_CONFIG_DIR`.

## Troubleshooting (setup/ping on Windows + WSL)

- `tabctl setup` fails with `Windows setup verification failed`: check `data.verification.reason` in JSON output (`ping-timeout`, `socket-not-found`, `socket-refused`, `ping-not-ok`, `extension-id-mismatch`), then follow printed manual steps.
- Runtime ID mismatch (`extension-id-mismatch`): compare expected vs runtime IDs from setup output, then rerun setup with the runtime ID shown by `edge://extensions` / `chrome://extensions`:
  - `tabctl setup --browser <edge|chrome> --extension-id <runtime-id>`
- Runtime command runs can auto-sync extension files when host/extension versions drift; rerun `tabctl query 'mutation { reloadExtension { reloading } }'` if the browser does not pick up changes immediately.
- For local release-like testing while developing, force runtime sync behavior with `TABCTL_AUTO_SYNC_MODE=release-like`.
- Disable runtime sync entirely with `TABCTL_AUTO_SYNC_MODE=off`.
- `tabctl ping --json` is a host connectivity/health check; use it to confirm the native host is reachable and healthy.
- Version metadata is intentionally health-only: regular GraphQL payloads do not include version fields unless you explicitly query health surfaces, which may expose fields such as `versionsInSync`, `hostBaseVersion`, and `baseVersion`.
- `tabctl ping` returns connect errors (`ENOENT`, `ECONNREFUSED`, timeout): ensure extension is loaded and active, rerun `tabctl setup`, and in WSL verify the profile data dir contains a current `pipe-endpoint` file.
- `tabctl doctor --fix --json` includes per-profile connectivity diagnostics in `data.profiles[].connectivity`; if ping remains unhealthy after local repairs, follow `manualSteps`.

Local release-like sync test recipe:
```bash
# 1) Install an older extension release into managed extension path
tabctl setup --browser edge --extension-id <extension-id> --release-tag v0.5.2

# 2) Run the current binary with forced release-like auto-sync
TABCTL_AUTO_SYNC_MODE=release-like cargo run --manifest-path rust/Cargo.toml -p tabctl -- query '{ tabs { total } }'

# 3) Verify host connectivity/health after auto-sync
tabctl ping --json
```

## Multi-Browser Setup

> **Advanced topic** — you only need this if you run tabctl with more than one browser (e.g. Edge *and* Chrome).

tabctl supports multiple browser profiles. Each profile connects to a different **browser** (Chrome, Edge).

<!-- test: "setup writes native host manifest", "setup writes native host manifest for chrome", "setup --name creates custom-named profile", "profile-list with multiple profiles shows all", "profile-switch success updates default", "--profile flag overrides active profile" -->
```bash
# Setup for Edge
tabctl setup --browser edge --extension-id <edge-extension-id>

# Setup for Chrome (with custom name)
tabctl setup --browser chrome --name chrome-work --extension-id <chrome-extension-id>

# List profiles
tabctl profile-list

# Switch default
tabctl profile-switch edge

# One-off command with different profile
tabctl --profile chrome-work query '{ tabs { total } }'
```

### Custom Chrome Profile Directories

If you launch Chrome with `--user-data-dir`, Chrome looks for native messaging manifests inside that directory. Use `--user-data-dir` in setup to write the manifest to the right place:

<!-- test: "setup --user-data-dir writes manifest to custom path" -->
```bash
tabctl setup --browser chrome --user-data-dir /path/to/chrome-profile
```

This writes the manifest to `<user-data-dir>/NativeMessagingHosts/` instead of the system-wide location.

### How It Works

Each profile gets its own:
- Native host manifest and wrapper script
- Unix socket for CLI-host communication
- Undo history log
- Data directory

Policy is shared across all profiles.

## Security
- The native host is locked to your extension ID.
- All data stays local; no external API keys are used.
- WSL ↔ Windows communication uses a local named-pipe bridge via `powershell.exe`; no TCP fallback is used on that path.

## Development

### Build workflow
The single `tabctl` binary is built from the Rust workspace (`rust/`). TypeScript is limited to the browser extension boundary (`src/extension/`). No Node.js or Go is required at runtime.

Build and verify:

```bash
cargo build --release -p tabctl   # build the binary
npm install && npm run build      # full pipeline (extension + Rust)
npm test                          # unit tests
```

Rust-only validation:
```bash
npm run rust:verify
npm run check:targets  # local cross-target cfg/type check
```

On macOS, `npm run check:targets` can use Zig for the C cross-compiler needed
by `libsqlite3-sys`:

```bash
brew install zig
```

The script auto-detects Zig outside CI and wires the Linux/Windows C compiler
environment for the check. The pre-push hook does not run this optional check;
run it manually when you want local cross-target coverage.

Browser-backed integration harness (requires built dist artifacts and Chrome):
```bash
npm run test:integration
```

WSL CI validates the WSL->Windows invocation bridge (`test.yml` `wsl` job) with phases: `prerequisites`, `diagnostics`, `build_and_unit`, `setup_validation`, `windows_invocation`, `integration`. Runtime/build execution is delegated to Windows commands (`cmd.exe`/`powershell.exe`), so WSL-local Rust compilation is not required.

### Versioning
The base version lives in `package.json` and is embedded into the CLI, host, and extension at build time.

Commands:
```bash
npm run bump:patch
npm run bump:minor
npm run bump:major
npm run bump:alpha
npm run bump:rc
npm run bump:stable
```

Pre-release staging flow:
- `bump:alpha` creates/increments `x.y.z-alpha.N`
- `bump:rc` promotes alpha to `x.y.z-rc.1` (or increments RC)
- `bump:stable` drops the prerelease suffix for final stable publish

Release automation:
- Run the **Prepare Release** workflow to choose or auto-detect the next version and open a release PR.
- When that PR merges, **Tag Release** creates `v<version>` and dispatches **Release**.
- The root `package.json` version and `optionalDependencies.tabctl-win32-x64` are kept in sync by `scripts/bump-version.js`.

Release publishing (`.github/workflows/release.yml`) supports both tag pushes and explicit workflow dispatch, and enforces:
- Git tag must match `package.json` version (`v<version>`)
- `package.json.optionalDependencies["tabctl-win32-x64"]` must match `package.json` version
- prerelease tags publish to `alpha`/`rc`; stable publishes to `latest`
- `npm run build` and `npm test` must pass before publish
- release assets include `tabctl-extension.zip` plus `tabctl-extension.zip.sha256`

Fetch the extension asset from a release with:
```bash
tabctl extension-fetch --version 0.5.3
```

Local builds default to a dev version when a `.git` directory is present, appending the short SHA.
```bash
npm run build
```

This produces versions like `0.1.0-dev.abc12345` (and appends `.dirty` when the repo has uncommitted changes).

For release builds without SHA, set:
```bash
TABCTL_VERSION_MODE=release npm run build
```

Notes:
- Browser reads and mutations now go through GraphQL via `tabctl query`.
- Reports include short descriptions from page metadata and a fallback snippet.
- `inspectTabs` supports `page-meta` plus selector reads with `all`, `text`, `textMode`, `styleProps`, and attrs such as `html`, `value`, `count`, `box`, `styles`, `visible`, `enabled`, and `checked`.
- `readTabs` converts main-frame page HTML to Markdown with Kreuzberg `html-to-markdown`; per-tab `status`, `emptyReason`, `diagnostics`, and `error` distinguish empty pages, unsupported URLs, injection failures, conversion failures, timeouts, and cached fallbacks (`status: CACHED`, `cached: true`, `diagnostics.source: "cache"`).
- Cached fallbacks use a bounded profile-local open-tab HTML cache refreshed after successful reads, active tab switches, and quiescent active pages; diagnostics include cache age and match mode when cached content is used.
- `captureScreenshots` returns tile metadata and image data from GraphQL.
- `undoAction` accepts either an explicit `txid` or `latest: true`.
- `tabctl history --json` returns a top-level JSON array.
