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
`~/.tabarchive/tabarchive-host.sh`

If `node` is not on PATH for Edge, pass an explicit path:

```bash
TABARCHIVE_NODE=/usr/local/bin/node bash scripts/setup-native-host.sh <YOUR_EXTENSION_ID>
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
tabctl list
tabctl analyze --stale-days 30
tabctl analyze --stale-days 30 --github
tabctl analyze --stale-days 30 --tab 123 --github --progress
tabctl analyze --stale-days 30 --github --github-concurrency 4 --progress
tabctl analyze --stale-days 30 --github --github-concurrency 4 --github-timeout-ms 4000 --progress
tabctl inspect --tab 123 --signal page-meta --progress
tabctl inspect --tab 123 --signal github-state --signal-concurrency 4 --signal-timeout-ms 4000 --progress
tabctl inspect --tab 123 --signal selector --selector "price=.price" --signal-timeout-ms 1500 --progress
tabctl inspect --tab 123 --signal selector --signal-config ~/.config/tabctl/signals.json --progress
tabctl focus --tab 123
tabctl policy --init
tabctl archive --all
tabctl archive --window 3
tabctl close --tab 123 --confirm
tabctl report --format md --out /path/to/report.md
tabctl undo <txid>
tabctl history --limit 20
```

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

## Undo log
Undo records are stored at:
`~/.tabarchive/undo.jsonl`

## Security
- The native host is locked to your extension ID.
- All data stays local; no external API keys are used.
