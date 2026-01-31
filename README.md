# Tab Archive Helper (Edge macOS)

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

## 2) Register the native messaging host
Create the host manifest at:
`~/Library/Application Support/Microsoft Edge/NativeMessagingHosts/com.example.tabarchive.json`

Use this content (replace the path + extension ID):

```json
{
  "name": "com.example.tabarchive",
  "description": "Tab archive native host",
  "path": "/Users/<you>/develop/scripts/check-browser-tabs/host/host.js",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://<YOUR_EXTENSION_ID>/"]
}
```

Make the host executable:

```bash
chmod +x /Users/<you>/develop/scripts/check-browser-tabs/host/host.js
```

## 3) Run the CLI
The CLI connects to the host over a local UNIX socket. It only works when Edge is open and the extension is active.

```bash
node /Users/<you>/develop/scripts/check-browser-tabs/cli/tabctl.js list
```

## CLI commands

```bash
tabctl list
tabctl analyze --stale-days 30
tabctl archive --all
tabctl archive --window 3
tabctl close --apply <analysisId>
tabctl close --tab 123 --confirm
tabctl report --format md --out /path/to/report.md
tabctl undo <txid>
tabctl history --limit 20
```

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
