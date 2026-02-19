# tabctl

Every open tab is a thread you forgot to pull. Tabctl finds them all.

A command-line instrument for browser tab orchestration — list, search, group, archive, close, undo — wired into Edge or Chrome through a native messaging bridge. Built for humans who hoard tabs and the AI agents who clean up after them.

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

Give your coding agent eyes into the browser. One command and it learns the protocol.

```bash
tabctl skill
# or: npx skills add https://github.com/ekroon/tabctl --skill tabctl -a opencode -a github-copilot -a claude-code
```

## Safety

Nothing leaves your machine. No cloud. No telemetry. Just a socket between your terminal and your browser, quiet as rain on neon.

Every mutation is undoable — `tabctl undo` rewinds closes, archives, and group changes like they never happened. A configurable policy layer shields pinned tabs and protected domains from accidental destruction. You pull the trigger; tabctl keeps the safety on until you mean it.

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

### 2. Set up your browser

Run setup with your browser's extension ID — it writes the manifest, wrapper script, and registers the profile:

<!-- test: "setup explicit --extension-id overrides auto-derived ID" -->
```bash
tabctl setup --browser chrome --extension-id <your-extension-id>
```

This will:
1. Write the native messaging manifest and wrapper script
2. Register the browser profile in `profiles.json`
3. Download the version-pinned release extension asset (`tabctl-extension.zip` + `.sha256`) into the tabctl data directory
4. Copy the extension to a stable location (`~/.local/state/tabctl/extension/`)
5. Print the path for loading as an unpacked extension in `chrome://extensions`

Without `--extension-id`, setup only downloads the extension and outputs JSON (no manifest or wrapper writes).

> **Edge?** Use `--browser edge` and load from `edge://extensions` instead.
>
> **Cross-platform:** setup works on macOS, Linux, and Windows. On Windows, setup verifies connectivity after writing setup artifacts and checks the runtime extension ID reported by the browser. Connectivity failures and runtime extension ID mismatches exit non-zero and print manual recovery steps (including expected vs runtime IDs).

Optional setup release overrides:
- Flags: `--release-repo`, `--release-tag` (or `--release-version`), `--release-asset`, `--skip-extension-download`
- Env vars: `TABCTL_RELEASE_REPO`, `TABCTL_RELEASE_TAG`, `TABCTL_RELEASE_ASSET`, `TABCTL_SETUP_FETCH_EXTENSION=0`
- Precedence: flags override env vars, then built-in defaults; if download fails, setup continues and includes warning details in setup output.

### 3. Verify and explore

<!-- test: "ping sends ping action", "list sends list action" -->
```bash
tabctl ping       # check the connection
tabctl list       # see your open tabs
```

> **Multiple browsers?** See [Multi-Browser Setup](#multi-browser-setup) for running tabctl with both Chrome and Edge.

## Commands

<!-- test: "list sends list action", "analyze passes tab ids and progress option", "inspect passes signal options", "close without confirm fails", "report format md returns markdown content", "undo sends undo action with txid" -->
| Command | Description |
|---------|-------------|
| `tabctl list` | List open tabs and groups |
| `tabctl open --url <url> --group <name>` | Open tabs into a group (reuses existing, skips duplicates) |
| `tabctl analyze` | Find stale or duplicate tabs |
| `tabctl inspect --tab <id>` | Extract page metadata or CSS selectors |
| `tabctl group-gather` | Merge duplicate groups with the same name |
| `tabctl close --tab <id>` | Close tabs with full undo support |
| `tabctl report` | Generate reports in JSON, Markdown, or CSV |
| `tabctl undo` | Revert the last action |

See [CLI.md](CLI.md) for the full command reference, options, and examples.

## Screenshot output
When `--out` is omitted, screenshots are written to `./.tabctl/screenshots/<timestamp>` and the JSON response includes `writtenTo`.

## Agent workflow (context -> selector)
Use screenshots only when you need visual context, then extract selectors with `inspect`.

1) Capture context (full page tiles):
<!-- test: "screenshot passes capture options" -->
```bash
tabctl screenshot --tab <id> --mode full
```

2) Identify the element visually, then extract its selector:
<!-- test: "inspect passes signal options" -->
```bash
tabctl inspect --tab <id> --signal selector --selector '{"name":"target","selector":".your-selector"}'
```

3) If you need an absolute URL, set `--selector-attr href-url` or set `attr` to `href-url`/`src-url`:
<!-- test: "inspect passes selector attr" -->
```bash
tabctl inspect --tab <id> --signal selector --selector '{"name":"link","selector":"a[href]","attr":"href-url"}'
tabctl inspect --tab <id> --signal selector --selector "link=a[href]" --selector-attr href-url
```

## Agent skills

Install the tabctl skill for agents (OpenCode, Claude Code, Codex, etc.) via the bundled command (uses the Skills CLI under the hood):

<!-- test: "skill install creates project skill link" -->
```bash
tabctl skill
```

This writes a project-local skill to `.opencode/skills/tabctl/SKILL.md`. You can also install globally:

<!-- test: "skill install supports global scope" -->
```bash
tabctl skill --global
```

To install into a specific agent toolchain with `skills`:

```bash
npx skills add https://github.com/ekroon/tabctl --skill tabctl -a opencode
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
- Profile registry: `<configDir>/profiles.json`
- WSL TCP port file: `<dataDir>/tcp-port` (written by the Windows host)

## Windows + WSL transport

On Windows, the host exposes a dual endpoint model:
- Windows native clients use a named pipe endpoint (`\\.\pipe\tabctl-<hash>`).
- WSL/Linux clients use `tcp://127.0.0.1:<port>`, with the host writing `<dataDir>/tcp-port`.

WSL endpoint discovery (CLI):
1. `TABCTL_SOCKET` (explicit endpoint); if this is a pipe endpoint in WSL, CLI still prefers discovered TCP.
2. `TABCTL_TCP_PORT` (forces `127.0.0.1:<port>`).
3. `tcp-port` file discovery from resolved data dir (and equivalent `/mnt/c/Users/*/.../tabctl/.../tcp-port` locations).
4. Fallback: `tcp://127.0.0.1:38000`.

Relevant knobs: `TABCTL_SOCKET`, `TABCTL_TCP_PORT`, `TABCTL_PROFILE`, `TABCTL_DATA_DIR`, `TABCTL_STATE_DIR`, `TABCTL_CONFIG_DIR`.

## Troubleshooting (setup/ping on Windows + WSL)

- `tabctl setup` fails with `Windows setup verification failed`: check `data.verification.reason` in JSON output (`ping-timeout`, `socket-not-found`, `socket-refused`, `ping-not-ok`, `extension-id-mismatch`), then follow printed manual steps.
- Runtime ID mismatch (`extension-id-mismatch`): compare expected vs runtime IDs from setup output, then rerun setup with the runtime ID shown by `edge://extensions` / `chrome://extensions`:
  - `tabctl setup --browser <edge|chrome> --extension-id <runtime-id>`
- `tabctl ping` returns connect errors (`ENOENT`, `ECONNREFUSED`, timeout): ensure extension is loaded and active, rerun `tabctl setup`, and in WSL verify `TABCTL_TCP_PORT` or `<dataDir>/tcp-port` matches a listening localhost port.

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
tabctl list --profile chrome-work
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
- TCP connections (used for WSL ↔ Windows communication) are secured with a per-session auth token. The host generates a random token on startup; the CLI reads it automatically. See [CLI.md](CLI.md) for details.
- TCP transport is available on all platforms via `TABCTL_HOST_TCP=1` (host) and `TABCTL_TRANSPORT=tcp` (CLI). All TCP connections are authenticated. See [CLI.md](CLI.md) for details.

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
```

Integration script (currently Rust-suite parity in CI/local):
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

Release publishing (`.github/workflows/publish.yml`) enforces:
- Git tag must match `package.json` version (`v<version>`)
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
- `close --apply` uses the most recent analysis by `analysisId`.
- `close` without `--apply` requires `--confirm` to prevent accidental closure.
- Reports include short descriptions from page metadata and a fallback snippet.
- `list` and `group-list` paginate by default (limit 100); use `--limit`, `--offset`, or `--no-page`.
- Use `--group-id -1` or `--ungrouped` to target ungrouped tabs.
- `--selector` implies `--signal selector`.
- Unknown inspect signals are rejected (valid: `page-meta`, `selector`).
- Selector `attr` supports `href-url`/`src-url` to return absolute http(s) URLs.
- `screenshot --out` writes per-tab folders into the target directory.
- `tabctl undo` accepts a positional txid, `--txid`, or `--latest`.
- `tabctl history --json` returns a JSON array in `data`.
- `--format` is only supported by `report` (use `--json` elsewhere).
