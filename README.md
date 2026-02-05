# Tab Control

Tab Control is a local-first Microsoft Edge Manifest V3 (MV3) extension with a native messaging host and the `tabctl` command-line interface (CLI) for inspecting, analyzing, and managing browser tabs. It can list tabs and groups, analyze duplicates or stale tabs, inspect page metadata and selector signals (for example, extracting a price or headline), open new tabs into named groups, move tabs and groups around, close or archive targets, generate reports in JSON/Markdown/CSV, and undo actions when needed. A policy file can mark pinned tabs or specific group titles as protected so automated actions skip them.

It requires the Edge extension to be installed and the native host running on the same machine; the CLI talks to the host, and the host proxies requests to the extension.

This repo contains:
- Edge MV3 extension (tab/group inspection + actions)
- Native messaging host (Node)
- CLI (`tabctl`) for on-demand workflows

The host only runs while Edge is open and the extension is connected.

## TypeScript workflow
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

## Versioning
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

## 1) Load the extension
1. Open `edge://extensions` and enable **Developer mode**.
2. Click **Load unpacked** and select `extension`.
3. Copy the extension ID shown on the extensions page.

## 2) Register the native messaging host (macOS)
Use the CLI to generate the manifest and wrapper:

```bash
tabctl setup --browser edge --extension-id <YOUR_EXTENSION_ID>
```

You can also use the helper script:

```bash
bash scripts/setup-native-host.sh <YOUR_EXTENSION_ID>
```

This writes the manifest to:
`~/Library/Application Support/Microsoft Edge/NativeMessagingHosts/com.erwinkroon.tabctl.json`

The manifest points to a wrapper script at:
`$XDG_STATE_HOME/tabctl/tabctl-host.sh` (or `~/.local/state/tabctl/tabctl-host.sh`)

If `node` is not on PATH for Edge, pass an explicit path:

```bash
TABCTL_NODE=/usr/local/bin/node bash scripts/setup-native-host.sh <YOUR_EXTENSION_ID>
```

## 3) Run the CLI
The CLI connects to the host over a local UNIX socket. It only works when Edge is open and the extension is active.

```bash
node /Users/<you>/develop/scripts/check-browser-tabs/cli/tabctl.js list
```

## CLI commands

```bash
tabctl --help
tabctl help --json
tabctl skill
tabctl list
tabctl list --limit 100
tabctl list --group-id -1
tabctl list --ungrouped
tabctl analyze --stale-days 30
tabctl analyze --ungrouped
tabctl analyze --stale-days 30 --github
tabctl analyze --stale-days 30 --tab 123 --github --progress
tabctl analyze --stale-days 30 --github --github-concurrency 4 --progress
tabctl analyze --stale-days 30 --github --github-concurrency 4 --github-timeout-ms 4000 --progress
tabctl dedupe --stale-days 30 --github
tabctl dedupe --ungrouped
tabctl inspect --tab 123 --signal page-meta --progress
tabctl inspect --tab 123 --limit 100
tabctl inspect --tab 123 --signal github-state --signal-concurrency 4 --signal-timeout-ms 4000 --progress
tabctl inspect --tab 123 --signal selector --selector "price=.price" --signal-timeout-ms 1500 --progress
tabctl inspect --tab 123 --signal page-meta --wait-for dom --wait-timeout-ms 8000
tabctl inspect --tab 123 --signal selector --signal-config ~/.config/tabctl/signals.json --progress
tabctl inspect --tab 123 --selector "price=.price"
tabctl inspect --tab 123 --selector '{"name":"cta","selector":"a.cta","attr":"href-url"}'
tabctl inspect --tab 123 --signal selector --selector '{"name":"price","selector":".price","text":"€","textMode":"contains"}'
tabctl screenshot --tab 123 --mode viewport
tabctl screenshot --tab 123 --mode full --tile-max-dim 1500 --max-bytes 2000000
tabctl screenshot --tab 123 --mode full --wait-for load --wait-timeout-ms 8000
tabctl focus --tab 123
tabctl refresh --tab 123
tabctl reload-extension
tabctl open --new-window --url https://example.com
tabctl open --url https://example.com --group "Docs" --color blue
tabctl open --url https://example.com --after-tab 123
tabctl open --window new --url https://example.com
tabctl move-tab --tab 123 --new-window
tabctl merge-window --from 1 --to 2
tabctl group-list
tabctl group-list --limit 100
tabctl group-update --group "Work" --title "Work Items" --color red --collapsed
tabctl group-ungroup --group "Work"
tabctl group-assign --tab 123 --group "Work" --create
tabctl policy --init
tabctl archive --all
tabctl archive --window 3
tabctl archive --ungrouped
tabctl close --tab 123 --confirm
tabctl close --ungrouped --confirm
tabctl report --format md --out /path/to/report.md
tabctl report --limit 100
tabctl undo <txid>
tabctl undo --latest
tabctl history --limit 20
```

## Screenshot output
When `--out` is omitted, screenshots are written to `./.tabctl/screenshots/<timestamp>` and the JSON response includes `writtenTo`.

## Agent workflow (context -> selector)
Use screenshots only when you need visual context, then extract selectors with `inspect`.

1) Capture context (full page tiles):
```bash
tabctl screenshot --tab <id> --mode full
```

2) Identify the element visually, then extract its selector:
```bash
tabctl inspect --tab <id> --signal selector --selector '{"name":"target","selector":".your-selector"}'
```

3) If you need an absolute URL, set `--selector-attr href-url` or set `attr` to `href-url`/`src-url`:
```bash
tabctl inspect --tab <id> --signal selector --selector '{"name":"link","selector":"a[href]","attr":"href-url"}'
tabctl inspect --tab <id> --signal selector --selector "link=a[href]" --selector-attr href-url
```

## Agent skills

Install the tabctl skill for agents (OpenCode, Claude Code, Codex, etc.) via the bundled command (uses the Skills CLI under the hood):

```bash
tabctl skill
```

This writes a project-local skill to `.opencode/skills/tabctl/SKILL.md`. You can also install globally:

```bash
tabctl skill --global
```

To install into a specific agent toolchain with `skills`:

```bash
npx skills add https://github.com/ekroon/tabctl --skill tabctl -a opencode
```

## Playwright MCP setup (extension + CLI testing)

Playwright MCP can drive Edge/Chrome to exercise the extension and `tabctl` CLI in a controlled window. Use the standard MCP config with the Edge browser channel and a dedicated profile so the extension and CLI live in the same browser state.

### 1) Install Playwright MCP

Add the MCP server to your client using the standard config (or copy `config/playwright-mcp.json` and adapt the path). Example config:

```json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": [
        "@playwright/mcp@latest",
        "--browser",
        "msedge",
        "--user-data-dir",
        ".tabctl/playwright-profile"
      ]
    }
  }
}
```

Notes:
- `--browser msedge` keeps the MCP session aligned with Edge (same browser required by the extension).
- `--user-data-dir .tabctl/playwright-profile` isolates test state from your normal profile.

#### Copilot Coding Agent (CCA) config

Copilot Coding Agent uses a slightly different MCP config shape (adds `type` and `tools`). Merge the contents of `config/copilot-mcp.json` into `~/.copilot/mcp-config.json` (or copy it if you are starting fresh) to enable Playwright MCP for CCA runs:

```json
{
  "mcpServers": {
    "playwright": {
      "type": "local",
      "command": "npx",
      "tools": ["*"],
      "args": [
        "@playwright/mcp@latest",
        "--browser",
        "msedge",
        "--user-data-dir",
        ".tabctl/playwright-profile"
      ]
    }
  }
}
```

### 2) Load the extension into the MCP-driven Edge profile

1. Start the MCP server through your client.
2. Open `edge://extensions` in the MCP-managed Edge window (either manually or via `tabctl open --new-window --url edge://extensions` once `tabctl` is installed).
3. Enable **Developer mode**, click **Load unpacked**, and choose the `extension/` folder.
4. Copy the extension ID and run:

```bash
tabctl setup --browser edge --extension-id <YOUR_EXTENSION_ID>
```

### 3) Smoke-test the CLI + extension via MCP

Use the MCP session to open a safe test window and exercise the CLI. Example:

```bash
ts=$(date +%s)
tabctl open --new-window --url https://example.com --url https://example.org --group "TEST-Smoke-${ts}"
tabctl group-list --window <windowId>
tabctl screenshot --tab <tabId> --mode viewport
tabctl inspect --tab <tabId> --signal selector --selector "a[href]" --selector-attr href-url
```

### Reload the extension after changes

After rebuilding the extension (`npm run build`), you can reload it without visiting the extensions page:

```bash
tabctl reload-extension
```

### Devbox/CI setup (launch with extension loaded)

To automate extension loading in a devbox or CI environment, launch the browser with the extension preloaded and a remote debugging port:

```bash
npm install
npm run build
bash scripts/launch-extension-browser.sh
```

Then point Playwright MCP to the running browser via CDP (sample config in `config/playwright-mcp-cdp.json`):

```json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": [
        "@playwright/mcp@latest",
        "--cdp-endpoint",
        "http://127.0.0.1:9222"
      ]
    }
  }
}
```

CI note: run the launch script under `xvfb-run -a` and ensure a Chrome/Edge binary is installed (set `TABCTL_BROWSER_BIN` if needed).

### Offline mock host (no Edge/extension)

If Edge or the extension is unavailable, you can still exercise the CLI with a lightweight mock host:

```bash
node scripts/mock-host.js --socket /tmp/tabctl-mock.sock
TABCTL_SOCKET=/tmp/tabctl-mock.sock tabctl open --new-window --url https://example.com --group "TEST-Mock"
TABCTL_SOCKET=/tmp/tabctl-mock.sock tabctl list
```

The mock host simulates responses (it does not control a real browser), but it lets you verify CLI flows and JSON output.

For more MCP configuration options, see the official Playwright MCP README: https://github.com/microsoft/playwright-mcp.

## Policy (protect tabs)
By default the CLI loads a policy file from:
`$XDG_CONFIG_HOME/tabctl/policy.json` (or `~/.config/tabctl/policy.json`)

This is a **protection-only** policy that marks tabs as ineligible for agent actions.
Example:

```json
{
  "protect": {
    "pinned": true,
    "groupTitles": ["\ud83d\udd12"]
  }
}
```

Create a default policy file:

```bash
tabctl policy --init
```

The shell setup script also installs a default policy if none exists; `tabctl setup` does not.
See `config/policy.example.json` for a starter template.

## Install tabctl on PATH
Use npm to install the local bin:

```bash
npm link
```

Then you can run `tabctl` directly.

Notes:
- `close --apply` uses the most recent analysis by `analysisId`.
- `close` without `--apply` requires `--confirm` to prevent accidental closure.
- Reports include short descriptions from page metadata and a fallback snippet.
- `list` and `group-list` paginate by default (limit 100); use `--limit`, `--offset`, or `--no-page`.
- Use `--group-id -1` or `--ungrouped` to target ungrouped tabs.
- `--selector` implies `--signal selector`.
- Unknown inspect signals are rejected (valid: `page-meta`, `github-state`, `selector`).
- Selector `attr` supports `href-url`/`src-url` to return absolute http(s) URLs.
- `screenshot --out` writes per-tab folders into the target directory.
- `tabctl undo` accepts a positional txid, `--txid`, or `--latest`.
- `tabctl history --json` returns a JSON array in `data`.
- `--format` is only supported by `report` (use `--json` elsewhere).

## Runtime state
- Socket: `$XDG_STATE_HOME/tabctl/tabctl.sock` (or `~/.local/state/tabctl/tabctl.sock`)
- Undo log: `$XDG_STATE_HOME/tabctl/undo.jsonl` (or `~/.local/state/tabctl/undo.jsonl`)

## Security
- The native host is locked to your extension ID.
- All data stays local; no external API keys are used.
