---
name: tabctl
description: Manage and analyze Edge tabs and groups with tabctl. Use when asked to list, search, analyze stale or duplicate tabs, generate reports, or organize tabs. Prefer read-only commands and only run mutating actions with explicit targets and confirmation.
---

# Tab Control

Use tabctl to inspect and analyze tabs safely, then perform targeted actions only when requested.

## Safety

- Prefer read-only commands: list, analyze, inspect, report.
- Never run `archive --all` or `close --apply` in a normal session.
- Only mutate explicit targets (`--tab`, `--group`, `--window`) and use `--confirm` for close.
- Respect policy: protected tabs are excluded.
- Use screenshots only when you need visual context.

## Discover commands

- Use `tabctl help` (or `tabctl help --json`) to discover commands and flags.
- For specific commands, use `tabctl <command> --help`.

## Common tasks

- List tabs in a window: `tabctl list --window <id|active|last-focused>`
- List ungrouped tabs: `tabctl list --ungrouped`
- Refresh a tab: `tabctl refresh --tab <id>`
- Generate a report: `tabctl report --format md` (add scope flags as needed)
- Get page metadata: `tabctl inspect --tab <id> --signal page-meta`
- Get page metadata after JS loads: `tabctl inspect --tab <id> --signal page-meta --wait-for settle`
- Extract links safely (absolute http(s) only): `tabctl inspect --tab <id> --selector '{"name":"links","selector":"a[href]","attr":"href-url","all":true}'`
- Capture visual context when needed: `tabctl screenshot --tab <id> --mode full`
- Undo most recent change: `tabctl undo --latest`
- Undo by txid: `tabctl undo <txid>` (from `tabctl history --json | jq -r '.data[] | .txid'`)

## Filter results (jq / node)

When you need custom filtering, pipe the JSON output to jq or node.

- JSON output shape (list): `.data.windows[].tabs[]`
- Stale candidates only (jq): `tabctl analyze --stale-days 7 | jq '.data.candidates[] | select(.reasons | any(.type == "stale")) | {tabId,title,url}'`
- Stale candidates only (node): `tabctl analyze --stale-days 7 | node -e 'const fs=require("fs"); const data=JSON.parse(fs.readFileSync(0,"utf8")); const stale=(data.data?.candidates||[]).filter(c=> (c.reasons||[]).some(r=>r.type==="stale")); console.log(JSON.stringify(stale,null,2));'`
- List tabs (jq): `tabctl list --json | jq -r '.data.windows[].tabs[] | select(.url | contains("devportal")) | {tabId,title,url}'`
- Search tabs by URL (jq): `tabctl list --json | jq '.data.windows[].tabs[] | select(.url | test("zoom"; "i")) | {tabId,title,url}'`

## Narrow scope

Use one or more of: `--window`, `--group`, `--group-id`, `--tab`, `--ungrouped`.
If scope is unclear, ask for it before running mutating commands.

## Output flags

- Use `--json` for JSON output (list/analyze/inspect/etc.).
- `--format` is only for `report` (e.g., `tabctl report --format md`).

## Wait modes for inspect/screenshot

Use `--wait-for` to control when inspection runs:

- `load` – wait for page load event (may miss JS-set titles)
- `dom` – wait for DOMContentLoaded
- `settle` – wait for URL and title to stabilize (500ms quiet period); use for JS-heavy pages or freshly opened tabs
- `none` – no waiting (default)

Example for newly opened tabs:
```bash
tabctl open --url "https://example.com" --json
tabctl inspect --tab <new_tab_id> --wait-for settle --wait-timeout-ms 10000 --json
```
