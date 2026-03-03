# tabctl CLI

## Quick start
```bash
mise use -g github:ekroon/tabctl   # or: cargo install --path rust/crates/tabctl
tabctl --help
tabctl help --json
tabctl policy --init
```

Runtime architecture: single `tabctl` Rust binary for CLI and native messaging host (`rust/crates/*`), with TypeScript limited to the extension boundary (`src/extension`).

## Configuration

### Config directory
`TABCTL_CONFIG_DIR` → `$XDG_CONFIG_HOME/tabctl` → `~/.config/tabctl`

Set `TABCTL_CONFIG_DIR` to override where tabctl looks for `config.json` and `policy.json`.

### config.json
Optional file in the config directory. Supported fields:

| Field | Description |
|-------|-------------|
| `dataDir` | Override the data directory (socket, undo log, host wrapper) |

### Data directory resolution
1. `config.json` → `dataDir` (if set)
2. `<configDir>/data/` (when `TABCTL_CONFIG_DIR` is set)
3. `$XDG_STATE_HOME/tabctl` → `~/.local/state/tabctl`

## Policy (enforced when present)
- Policy file: `<configDir>/policy.json` (default: `~/.config/tabctl/policy.json`)
- If the file is missing, no policy is applied.
- Protected tabs are excluded from outputs and actions.
- The default policy protects pinned tabs and group title `🔒`.

Create the default policy file:
```bash
tabctl policy --init
```

## Global flags
- `--help` / `-h`: command-specific help
- `--version` / `-v`: show version and exit
- `--json`: JSON output
- `--pretty`: pretty-print JSON (default: true)
- `--no-pretty`: disable pretty-printing JSON
- `--progress`: enable progress reporting for commands that support progress events
- `--profile <name>`: override active profile for this command
- `--format` is only supported by `report` (use `--json` elsewhere)

## Output contract (compact by default)

tabctl output is compact by default so automation can depend on stable, minimal response shapes. By default, JSON focuses on identifiers, scope, and action results; richer fields are exposed through command-specific options instead of a global verbose/full mode.

### Verbose/full retrieval paths (by command family)
- `analyze`, `dedupe`: use `--window-title` to include active window title context in output.
- `inspect`: request additional metadata via `--signal` (`page-meta`, `selector`) and `--selector` options.
- `screenshot`: use `--mode full` for tiled full-page capture output (default remains viewport).
- `report`: select `--format json|md|csv` depending on downstream detail needs.
- `ping`: use `--json` for runtime/version sync fields (`versionsInSync`, `hostBaseVersion`, `baseVersion`).

There is no global `--verbose` or `--full` flag today; use the command-local options above.

### Migration notes (known output-shape changes)
- `list --json` is window-nested (`windows[].tabs[]`); update parsers that assumed a flat top-level tabs array.
- `history --json` returns a top-level array (`.[]`).
- Non-health commands (`open`, `list`, etc.) no longer carry version metadata; use `tabctl ping --json` for version/sync fields.
- `list` and `group-list` now paginate by default (limit 100); add `--limit`, `--offset`, or `--no-page` where full result sets are required.

## Option Groups

Commands reference these reusable option groups to avoid documentation duplication.

### Scope Options
Filter which tabs/groups to operate on.

| Option | Description |
|--------|-------------|
| `--tab <id>` | Target specific tab(s) by ID (repeatable) |
| `--group <name>` | Target tabs in group by title |
| `--group-id <id>` | Target group by ID (use `-1` for ungrouped) |
| `--ungrouped` | Alias for `--group-id -1` |
| `--window <id|active|last-focused>` | Target tabs in specific window |
| `--all` | Target all eligible tabs |

### Pagination Options
Control result paging (default limit: 100).

| Option | Description |
|--------|-------------|
| `--limit <n>` | Maximum items to return |
| `--offset <n>` | Skip first n items |
| `--no-page` | Disable pagination, return all results |

These option groups apply anywhere referenced; they are not repeated under every command.

## Commands

### help
Show CLI help.
```bash
tabctl help
tabctl help --json
tabctl help open
```

### list
List browser tabs.
```bash
tabctl list
```
JSON output is nested under `windows[].tabs[]` when using `--json`.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--groups` (alias for group-list command)

### analyze
Find duplicates and stale tabs.

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--stale-days <n>`
- `--window-title` (include active window title in output)
- `--progress`

If no scope is provided, all eligible tabs are analyzed.

### dedupe
Plan (and optionally close) duplicate tabs.

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--stale-days <n>`
- `--include-stale`
- `--window-title` (include active window title in output)
- `--progress`
- `--confirm`

### inspect
Run signals to collect metadata (page-meta, selector).

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--signal-config <path>`
- `--signal <id>` (repeatable)
- `--selector <name=css|json>` (repeatable; supports `text`/`textMode` in JSON)
- `--selector-attr <attr>` (default attr for selectors)
- `--signal-concurrency <n>`
- `--signal-timeout-ms <ms>`
- `--wait-for load|dom|settle|none`
- `--wait-timeout-ms <ms>`
- `--progress`

Wait modes:
- `load` – wait for page load event
- `dom` – wait for DOMContentLoaded
- `settle` – wait for URL and title to stabilize (500ms after load); recommended for JS-heavy pages
- `none` – no waiting

Signals:
- `page-meta` (description + h1)
- `selector` (runtime-configured selectors)

Notes:
- `--selector` implies `--signal selector`.
- Unknown signals are rejected; valid signals: `page-meta`, `selector`.
- Selector `attr` supports `href-url`/`src-url` to return absolute http(s) URLs.
- Selector `:contains()` is not supported; use selector text filters or screenshots.

Suggested flow for agents:
1. `tabctl screenshot --tab <id> --mode full`
2. Identify the element visually.
3. `tabctl inspect --tab <id> --signal selector --selector '{"name":"target","selector":".your-selector"}'`
4. For links, set `--selector-attr href-url` (or per-selector `attr: "href-url"`).

### screenshot
Capture screenshots for tabs (viewport or full-page tiles).

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--mode viewport|full`
- `--format png|jpeg`
- `--quality <n>` (jpeg only)
- `--tile-max-dim <px>` (full mode only)
- `--max-bytes <n>`
- `--wait-for load|dom|settle|none`
- `--wait-timeout-ms <ms>`
- `--out <dir>` (writes per-tab folders)
- `--progress`

Output:
- Writes files to `./.tabctl/screenshots/<timestamp>` by default and includes `writtenTo` in JSON output.

Examples:
```bash
tabctl screenshot --tab 123 --mode viewport
tabctl screenshot --tab 123 --mode full --tile-max-dim 1500 --max-bytes 2000000
```


### focus
Focus a tab by id.
```bash
tabctl focus --tab <id>
```

### refresh
Refresh a tab by id.
```bash
tabctl refresh --tab <id>
```

### reload
Reload the extension's background service worker. Useful during development.
```bash
tabctl reload
```

### open
Open new tabs and optionally group them in a target window.

When `--group <name>` is provided, an existing group with that name is reused by default. Duplicate URLs already present in the target group are skipped. New groups are automatically positioned before ungrouped tabs.

Options:
- `--url <url>` (repeatable)
- `--group <name>` (group title; reuses an existing group if one matches)
- `--new-group` (force creation of a new group even if one with the same name exists)
- `--allow-duplicates` (open URLs even if already present in the target group)
- `--color <name>` (group color)
- `--before-tab <id>`
- `--after-tab <id>`
- `--after-group <name>` (insert tabs after this group)
- `--window <id|active|last-focused|new>`
- `--new-window`
- `--window-group <name>` (window containing a group with this title)
- `--window-tab <id>` (window containing this tab)
- `--window-url <substring>` (window containing a tab whose URL includes this substring)

Allowed colors: `grey`, `blue`, `red`, `yellow`, `green`, `pink`, `purple`, `cyan`, `orange`.

Examples:
```bash
# Open a URL into an existing "Docs" group (or create it)
tabctl open --url https://example.com --group "Docs" --color blue

# Force a new group even if "Docs" already exists
tabctl open --url https://example.com --group "Docs" --new-group

# Allow duplicate URLs in the group
tabctl open --url https://example.com --group "Docs" --allow-duplicates
```

If no window selector is provided, the focused window is used (fallback to last-focused if needed).

### group-gather
Merge duplicate groups with the same name within a window.
Options:
- `--window <id|active|last-focused>` (target window)
- `--group <name>` (group name to gather; gathers all duplicates if omitted)

Examples:
```bash
# Merge all duplicate-named groups in the active window
tabctl group-gather --window active

# Merge only groups named "Work"
tabctl group-gather --group Work --window 123
```

### group-list
List groups with window ids/labels and tab counts.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

### group
Alias for `group-list`.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

### group-update
Update group metadata (title, color, or collapsed state).
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles)
- `--title <name>`
- `--color <name>`
- `--collapsed`
- `--expanded`

### group-ungroup
Remove all tabs from a group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles)

### group-assign
Move existing tabs into an existing group (or create one).
Options:
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles or create target)
- `--create` (create group if missing)
- `--color <name>`
- `--collapsed`
- `--expanded`

### move-tab
Move a single tab before/after a tab or group.
Options:
- `--tab <id>`
- `--before-tab <id>`
- `--after-tab <id>`
- `--before-group <name>`
- `--after-group <name>`
- `--window <id|active|last-focused>` (disambiguate group names)
- `--new-window`

Note: `--before-group`/`--after-group` only position tabs; use `group-assign` to move tabs into a group.

### move-group
Move a group before/after a tab or group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--before-tab <id>`
- `--after-tab <id>`
- `--before-group <name>`
- `--after-group <name>`
- `--window <id|active|last-focused>` (disambiguate group names)
- `--new-window`

Note: `--before-group`/`--after-group` only position groups; they do not merge groups.

### merge-window
Move tabs from one window into another (policy-filtered).
Options:
- `--from <id>`
- `--to <id>`
- `--close-source` (requires `--confirm`)
- `--confirm`

### setup
Install the native host manifest, wrapper script, and register a profile. Works on macOS, Linux, and Windows.
Also attempts to download the version-matched release extension asset (`tabctl-extension.zip` + `.sha256`) into the tabctl data directory.
Setup can also sync from a local unpacked extension directory with `--extension-dir` (or `TABCTL_SETUP_EXTENSION_DIR`) for local development.
Options:
- `--browser edge|chrome` (required)
- `--extension-id <id>` (optional; explicit override of the derived extension ID)
- `--extension-dir <path>` (optional; local unpacked extension directory, e.g. `dist/extension`)
- `--user-data-dir <path>` (optional; write manifest to a custom Chrome/Edge profile directory instead of the system-wide location)
- `--name <name>` (optional; profile name, defaults to browser name)
- `--release-repo <owner/repo>` (optional; or `TABCTL_RELEASE_REPO`)
- `--release-tag <tag>` / `--release-version <version>` (optional; or `TABCTL_RELEASE_TAG`)
- `--release-asset <name>` (optional; or `TABCTL_RELEASE_ASSET`)
- `--skip-extension-download` (optional; or `TABCTL_SETUP_FETCH_EXTENSION=0`)
- `--dev` (coming soon; dev/CI mode via CDP)

By default, setup derives the extension ID from the managed extension path and runs the full setup path. Use `--extension-id` to override the derived ID.
Managed extension path for load-unpacked workflows: `<dataDir>/extension/`.

Each run creates or updates a profile in `profiles.json`. The first profile registered becomes the default.
Release override precedence: CLI flags take precedence over environment variables, then built-in defaults.
If extension release download fails, setup continues and reports `extension_download_failed` in `data.warnings` with fallback path details.

Local dev example (no release download):
```bash
tabctl setup --browser edge --extension-dir dist/extension
```

On Windows, setup verifies host connectivity by default after writing setup artifacts and compares the browser-reported runtime extension ID with the expected ID. Connectivity failures and runtime extension ID mismatches exit non-zero with manual recovery steps (including expected vs runtime IDs).

Run once per browser:
```bash
tabctl setup --browser edge --extension-id mpglnmehddpkinfhheeahiicfieegcon
tabctl setup --browser chrome --name chrome-work --extension-id <your-extension-id>
```

### doctor
Diagnose and repair profile health. Checks each profile's native host artifacts and live `ping` connectivity.
Options:
- `--fix` auto-repair broken profiles

```bash
tabctl doctor              # show health status
tabctl doctor --fix        # auto-repair broken profiles
```

`--fix` repairs local profile artifacts, attempts extension sync to the current tabctl version, then re-runs connectivity checks. If ping is still unhealthy, doctor returns manual remediation steps under each profile's `connectivity.manualSteps`.

### policy
Show the current policy summary and path, or create a default policy file.
Options:
- `--init`

### archive
Move tabs/groups into the Archive window.

**Uses:** [Scope Options](#scope-options)

### close
Close explicit targets only (policy-filtered). Requires confirmation for direct close.

**Uses:** [Scope Options](#scope-options) (except `--all`)

Additional options:
- `--apply <analysisId>`
- `--confirm`
- `--dry-run` (alias for `analyze`)

Note: policy enforcement blocks `close --apply`; use explicit tab targets.

### report
Generate a report for eligible tabs.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--format json|md|csv`
- `--out <path>`

### undo
Restore the last action by transaction id.
```bash
tabctl undo <txid>
```
Options:
- `--txid <id>` (alias for positional)
- `--latest` (undo most recent transaction)

Examples:
```bash
tabctl undo tx-123
tabctl undo --txid tx-123
tabctl undo --latest
```

### history
List recent actions.
Options:
- `--limit <n>`
```bash
tabctl history --limit 20
```
Returns a top-level JSON array.

JSON example:
```bash
tabctl history --json | jq -r '.[] | {txid, action, summary}'
```

### extension-fetch
Download the release extension bundle for a specific version/tag.
Options:
- `--version <version|tag>` (defaults to current CLI version)
- `--repo <owner/repo>` (defaults to `ekroon/tabctl`)
- `--asset <name>` (defaults to `tabctl-extension.zip`)
- `--out <path>` (optional explicit output path)

```bash
tabctl extension-fetch --version 0.5.3
tabctl extension-fetch --version v0.5.3 --out /tmp/tabctl-extension.zip
```


### version
Show CLI version information.
```bash
tabctl version
```
Environment:
- `TABCTL_VERSION_MODE=release` (force release version without git sha)
- `TABCTL_VERSION_MODE=dev` (force dev version with git sha)
- `TABCTL_AUTO_SYNC_MODE=auto|release-like|off` (runtime extension auto-sync mode)

### profile-list
List configured profiles.
```bash
tabctl profile-list
```

### profile-show
Show active profile details (name, browser, data directory, socket path, etc.).
```bash
tabctl profile-show
tabctl profile-show --json
```

### profile-switch
Switch the default profile.
```bash
tabctl profile-switch <name>
```

### profile-remove
Remove a profile from the registry. Does not delete native host manifests.
```bash
tabctl profile-remove <name>
```

### ping
Check host/extension connectivity and runtime version sync status.
```bash
tabctl ping
```
Use `tabctl ping --json` as the canonical runtime version surface (`versionsInSync`, `hostBaseVersion`, `baseVersion`).
Non-health command outputs (`open`, `list`, etc.) intentionally do not include version metadata.

### host
Run as native messaging host (stdio mode). This subcommand is invoked automatically by the browser via the native messaging manifest — it is not typically run manually.

```bash
tabctl host
```

The browser launches `tabctl host` when the extension connects. It communicates over stdin/stdout using the Chrome native messaging protocol and listens on a Unix socket (or named pipe on Windows) for CLI commands.

## Windows + WSL endpoint model

Windows host runtime listens on both:
- Named pipe for native Windows clients (`\\.\pipe\tabctl-<hash>`)
- Localhost TCP for WSL/Linux clients (`tcp://127.0.0.1:<port>`)

The host publishes the WSL TCP port to `<dataDir>/tcp-port`.

WSL endpoint resolution order (CLI):
1. `TABCTL_SOCKET` (explicit endpoint; explicit pipe endpoints are translated to discovered TCP in WSL when available)
2. `TABCTL_TCP_PORT`
3. `tcp-port` discovery from resolved data dir and `/mnt/c/Users/*/.../tabctl/.../tcp-port`
4. Fallback `tcp://127.0.0.1:38000`

### Universal TCP Transport (opt-in)

TCP transport can be enabled on any platform, not just WSL. This is useful for:
- Remote debugging or headless setups
- Container-to-host communication
- Development and testing

**Host side — enable TCP listener:**

```bash
TABCTL_HOST_TCP=1 tabctl host
```

This starts a TCP listener on `127.0.0.1` alongside the primary transport (Unix socket or named pipe). The port is written to `<dataDir>/tcp-port`.

**CLI side — use TCP transport:**

```bash
TABCTL_TRANSPORT=tcp tabctl list --all
```

The CLI reads the port from the `tcp-port` file in the data directory. You can also specify a port directly:

```bash
TABCTL_TCP_PORT=38500 TABCTL_TRANSPORT=tcp tabctl ping
```

All TCP connections require a valid auth token (see TCP Transport Security below).

### TCP Transport Security

When the host listens on TCP (Windows always, other platforms via `TABCTL_HOST_TCP=1`), connections are secured with a shared auth token:

- The host generates a random token on startup and writes it to `<dataDir>/auth-token`
- The CLI reads this token and includes it in every TCP request
- Requests without a valid token are rejected with "Authentication failed"
- Unix socket and named pipe connections do not require a token (filesystem permissions provide security)

**Environment variable override:**

```bash
export TABCTL_AUTH_TOKEN=<token>
```

This overrides file-based token discovery, useful for custom setups.

**WSL path resolution:**

The CLI extracts the Windows username from `$PATH` (matching `/mnt/c/Users/<username>/`) to locate the auth token file in the Windows filesystem, rather than scanning all user directories.

Notes:
- Use `--group-id -1` or `--ungrouped` to target ungrouped tabs.
- `screenshot --out` writes per-tab folders into the target directory.
- When multiple groups share the same title, commands that target by `--group <name>` will error with suggestions to use `group-gather` to merge duplicates, `--group-id` to target by ID, or `--window` to narrow scope.
## Environment variables

| Variable | Description |
|----------|-------------|
| `TABCTL_CONFIG_DIR` | Override config directory (default: `$XDG_CONFIG_HOME/tabctl`) |
| `TABCTL_DATA_DIR` | Override resolved data directory |
| `TABCTL_EXTENSION_ID` | Extension ID for `setup` command |
| `TABCTL_PROFILE` | Override active profile (same as `--profile` flag) |
| `TABCTL_RELEASE_ASSET` | Override setup release asset filename |
| `TABCTL_RELEASE_REPO` | Override setup release repository (`owner/repo`) |
| `TABCTL_RELEASE_TAG` | Override setup release tag/version |
| `TABCTL_SETUP_EXTENSION_DIR` | Local unpacked extension directory for setup sync |
| `TABCTL_AUTH_TOKEN` | Override TCP auth token (skips file-based discovery) |
| `TABCTL_SOCKET` | Override socket endpoint (`unix://`, `pipe://`, `tcp://`) |
| `TABCTL_STATE_DIR` | Override state directory fallback (`$XDG_STATE_HOME/tabctl`) |
| `TABCTL_SETUP_FETCH_EXTENSION` | Set to `0` to skip setup extension release download |
| `TABCTL_AUTO_SYNC_MODE` | Runtime extension sync mode: `auto` (default), `release-like`, or `off` |
| `TABCTL_HOST_TCP` | Set to `1` to enable TCP listener on the host (all platforms) |
| `TABCTL_TRANSPORT` | Set to `tcp` to use TCP transport in the CLI |
| `TABCTL_TCP_PORT` | Force localhost TCP endpoint (WSL/Linux clients) |
| `TABCTL_VERSION_MODE` | `release` or `dev` for version output |

## Troubleshooting (setup/ping/runtime ID)

- Setup verification failures return `Windows setup verification failed`; inspect `data.verification.reason` (`ping-timeout`, `socket-not-found`, `socket-refused`, `ping-not-ok`, `extension-id-mismatch`) and follow `manualSteps`.
- For `extension-id-mismatch`, rerun setup with the runtime extension ID shown by the browser:
  - `tabctl setup --browser <edge|chrome> --extension-id <runtime-id>`
- `tabctl ping` connect failures (`ENOENT`/`ECONNREFUSED`/timeout) usually mean host/extension disconnect; reload extension, rerun setup, and in WSL confirm `<dataDir>/tcp-port` or `TABCTL_TCP_PORT` matches a listening `127.0.0.1` port.
- For local release-like sync testing while developing, force runtime behavior with `TABCTL_AUTO_SYNC_MODE=release-like` on a non-ping command (example: `TABCTL_AUTO_SYNC_MODE=release-like tabctl list --all`).
- Disable runtime sync checks with `TABCTL_AUTO_SYNC_MODE=off`.
- For profile-targeted diagnostics and remediation hints, run `tabctl doctor --fix --json` and inspect `data.profiles[].connectivity`.

## Profiles
Each `tabctl setup` run registers a profile in `<configDir>/profiles.json`. A profile stores the browser type, extension ID, and data directory. The first profile registered becomes the default.

Profile resolution order:
1. `--profile <name>` flag (or `TABCTL_PROFILE` env var)
2. Default profile from `profiles.json`
3. Legacy mode (no profiles configured)

Each profile gets its own data directory with a separate socket and undo log. Policy is shared across all profiles.

## Runtime state
- Socket: `<dataDir>/tabctl.sock` (default: `~/.local/state/tabctl/tabctl.sock`)
- Undo log: `<dataDir>/undo.jsonl` (default: `~/.local/state/tabctl/undo.jsonl`)
- Profile registry: `<configDir>/profiles.json`

See [Configuration](#configuration) for how the data directory is resolved.

## Build and release
- `npm run build`: generates version metadata, bundles the extension, and builds the Rust workspace (single `tabctl` binary).
- `npm test`: runs build + Rust formatting/lint/tests (`npm run rust:verify`).
- `npm run test:integration`: runs the browser-backed Rust integration harness (`cargo test --manifest-path rust/Cargo.toml --test browser_integration -- --ignored --nocapture`), which requires built dist artifacts and Chrome.

Release channel mapping:
- `x.y.z-alpha.N` → npm `alpha`
- `x.y.z-rc.N` → npm `rc`
- `x.y.z` → npm `latest`
