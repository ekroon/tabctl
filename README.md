# tabctl

Every open tab is a thread you forgot to pull. Tabctl finds them all.

A command-line instrument for browser tab orchestration — list, search, group, archive, close, undo — wired into Edge or Chrome through a native messaging bridge. Built for humans who hoard tabs and the AI agents who clean up after them.

## Install

```bash
npm install -g tabctl
tabctl setup --browser chrome
# Load the extension: chrome://extensions → Developer mode → Load unpacked → paste: ~/.local/state/tabctl/extension/
tabctl ping
```

If it pings back, the wire is live. You're connected.

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

`tabctl` works through a lightweight local stack: the CLI talks to a native messaging host, which proxies requests to a browser extension. The host only runs while the browser is open and the extension is connected.

This repo contains:
- Chrome/Edge extension (tab/group inspection + actions)
- Native messaging host (Node)
- CLI (`tabctl`) for on-demand workflows

## Quick Start

### 1. Build and install

```bash
npm install
npm run build
npm link          # puts tabctl on your PATH
```

If you haven't run `npm link`, you can always use `node ./cli/tabctl.js` instead of `tabctl`.

### 2. Set up your browser

Run setup — it syncs the extension, tells you where to load it, and auto-derives the extension ID:

<!-- test: "setup explicit --extension-id overrides auto-derived ID" -->
```bash
tabctl setup --browser chrome
```

This will:
1. Copy the extension to a stable location (`~/.local/state/tabctl/extension/`)
2. Print the path (and copy it to your clipboard)
3. Ask you to load it as an unpacked extension in `chrome://extensions`
4. Auto-derive the extension ID from the installed path

> **Edge?** Use `--browser edge` and load from `edge://extensions` instead.

If you need to override the auto-derived ID (e.g. for a custom extension path):

<!-- test: "setup writes native host manifest for chrome" -->
```bash
tabctl setup --browser chrome --extension-id <your-extension-id>
```

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

## Multi-Browser Setup

> **Advanced topic** — you only need this if you run tabctl with more than one browser (e.g. Edge *and* Chrome).

tabctl supports multiple browser profiles. Each profile connects to a different **browser** (Chrome, Edge).

<!-- test: "setup writes native host manifest", "setup writes native host manifest for chrome", "setup --name creates custom-named profile", "profile-list with multiple profiles shows all", "profile-switch success updates default", "--profile flag overrides active profile" -->
```bash
# Setup for Edge
tabctl setup --browser edge

# Setup for Chrome (with custom name)
tabctl setup --browser chrome --name chrome-work

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

## Development

### TypeScript workflow
Source lives in `src/` and compiles to `build/`, then syncs to the runtime locations:
- `src/extension/background.ts` -> `extension/background.js`
- `src/host/host.ts` -> `host/host.js`
- `src/cli/tabctl.ts` -> `cli/tabctl.js`
- `src/tests/unit/*.ts` -> `tests/unit/*.js`

Build and test:

```bash
npm install
npm run build
npm test
```

### Rust migration investigation

See [RUST_PORTING.md](RUST_PORTING.md) for a feasibility analysis and phased migration plan for porting the CLI and native host to Rust.

### Versioning
The base version lives in `package.json` and is embedded into the CLI, host, and extension at build time.

Commands:
```bash
npm run bump:patch
npm run bump:minor
npm run bump:major
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
