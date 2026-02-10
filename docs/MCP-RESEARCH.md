# MCP Support Research — tabctl

## Overview

This document evaluates adding Model Context Protocol (MCP) support to tabctl
via a `tabctl mcp` subcommand, analyzes library options, and proposes an
architecture that avoids maintaining two separate APIs.

---

## 1. Library Comparison

### Option A — `@modelcontextprotocol/sdk` (official)

| Aspect | Details |
|--------|---------|
| Package | `@modelcontextprotocol/sdk` + `zod` |
| Version | 1.26.0 (stable, Feb 2026) |
| CJS support | ✅ Works with `require()` — verified in this repo |
| Transports | stdio, Streamable HTTP, legacy SSE |
| API | `McpServer.tool(name, desc, zodSchema, handler)` |
| Maturity | Production-ready; v2 split planned but not stable yet |
| Size | ~91 packages in dependency tree |
| Maintenance | Anthropic-backed, active |

### Option B — `fastmcp`

| Aspect | Details |
|--------|---------|
| Package | `fastmcp` |
| CJS support | ✅ |
| API | `FastMCP.addTool({ name, description, parameters, execute })` |
| Extra features | Built-in auth, session tracking, health-check |
| Size | Depends on `@modelcontextprotocol/sdk` internally |
| Maintenance | Community (punkpeye) |

### Option C — Raw protocol (no library)

| Aspect | Details |
|--------|---------|
| Approach | Implement JSON-RPC 2.0 over stdio manually |
| CJS support | ✅ (no dependency) |
| Maintenance | Full burden on us |
| Risk | Protocol compliance drift, missing capabilities |

### Recommendation: **Official SDK (`@modelcontextprotocol/sdk`)**

- It's the protocol standard. Agents test against it.
- CJS `require()` works today (verified with Node 24 + our CommonJS tsconfig).
- FastMCP wraps the official SDK anyway — no benefit, extra layer.
- Raw protocol is fragile and would need constant maintenance.

---

## 2. Preventing Context Overflow

MCP tools return structured content to the agent. Large responses (e.g.,
`list --all` with hundreds of tabs, `report`, `screenshot` base64 data) can
blow up the agent's context window.

### Strategy: MCP tools return **compact summaries by default**

| Technique | Implementation |
|-----------|----------------|
| **Default pagination** | All list-like tools default to `limit: 20` (not CLI's 100) |
| **Truncation** | String fields (title, URL) capped at reasonable lengths |
| **No screenshots in MCP** | `screenshot` tool excluded — agents should use browser tools directly |
| **Structured output** | Return counts + key fields, not raw JSON dumps |
| **Scope required** | Require `windowId` or `groupTitle` for broad queries |
| **Result cap** | Hard limit: 50 items max per tool call |
| **Summary mode** | Tools like `analyze` return only summary counts + top-5 candidates |

Example — `list_tabs` returns:
```
Found 12 tabs in window 1234:
1. [Tab 5678] Example.com — https://example.com
2. [Tab 5679] GitHub — https://github.com/...
...
Page 1/1 (12 tabs)
```

Not a full JSON dump of the entire browser state.

### Hard limits

- Max output: 4000 characters per tool response
- Pagination: agent can request `offset` for more results
- Binary data (screenshots): excluded from MCP tools entirely

---

## 3. Preventing Two APIs — The "CLI-Through" Architecture

**Key insight**: The MCP server does NOT need its own handler logic. It
translates MCP tool calls into the same `sendRequest()` calls the CLI already
uses.

```
Agent ──MCP stdio──→ tabctl mcp ──socket──→ Host ──native msg──→ Extension
                       │
                       └── Uses same sendRequest() / client.ts as CLI
```

### How it works

1. Each MCP tool definition maps 1:1 to a CLI action
2. The MCP handler calls `sendRequest()` with the same params
3. Response is formatted for context-friendliness (compact text, not raw JSON)
4. **No new action handlers in host or extension**

### What we share

| Layer | Shared? |
|-------|---------|
| Socket client (`client.ts`) | ✅ Same `sendRequest()` |
| Action names | ✅ Same action strings |
| Param schemas | ✅ Same param objects |
| Response parsing | ❌ MCP formats for text; CLI uses JSON |
| Validation | ✅ Zod schemas mirror CLI param validation |

### What's different

- **Input**: Zod schemas instead of CLI arg parsing
- **Output**: Compact text formatted for LLM context instead of JSON dumps
- **Error handling**: MCP error responses instead of `process.exit(1)`

This means:
- **Zero changes to host or extension code**
- **Zero new actions or message types**
- **One source of truth for business logic (extension)**
- **CLI and MCP share the same transport layer**

---

## 4. MCP vs Skill — Cost/Benefit Analysis

### What we have today (Skill)

The `skills/tabctl/SKILL.md` provides agents with:
- Safety rules (read-only preference, scope-first)
- Command examples and patterns
- jq/node filtering recipes
- Wait-mode guidance

The skill works by injecting instructions into the agent's context. The agent
then runs CLI commands via bash/shell tool.

### What MCP adds

| Capability | Skill | MCP |
|------------|-------|-----|
| Discoverability | ❌ Agent needs skill pre-installed | ✅ Auto-discovered via MCP config |
| Type safety | ❌ String args, easy to mistype | ✅ Zod schemas, validated params |
| Context efficiency | ❌ Full JSON output in context | ✅ Compact, curated responses |
| Error handling | ❌ stderr + exit code | ✅ Structured MCP errors |
| Multi-agent support | ❌ Agent-specific skill install | ✅ Universal MCP protocol |
| IDE integration | ❌ None | ✅ VS Code, Cursor, Windsurf, etc. |
| No shell required | ❌ Needs bash tool | ✅ Direct tool invocation |
| Safety enforcement | ⚠️ Instructions only | ✅ Schema constraints prevent bad calls |
| Setup complexity | Low (copy file) | Medium (MCP config in client) |

### When to use which

- **Skill alone**: Fine for power users who already use tabctl CLI and want
  agent assistance. The agent gets rich instructions and can pipe/filter.
- **MCP alone**: Good for discoverability and multi-agent setups. Agents get
  structured tools without needing shell access.
- **Both** (recommended): Skill provides safety instructions and advanced
  patterns. MCP provides the structured tool interface. They complement each
  other.

### Verdict

MCP is worth implementing because:
1. **Discoverability** — any MCP-capable agent finds tabctl automatically
2. **Context efficiency** — we control response size, preventing overflow
3. **Safety** — schema validation prevents `close --apply` style accidents
4. **Ecosystem** — VS Code, Cursor, Claude Desktop, etc. all support MCP
5. **Low cost** — CLI-through architecture means minimal new code
6. **Keep the skill** — the skill provides nuanced instructions (workflow
   patterns, jq recipes) that MCP tools can't express

---

## 5. Proposed MCP Tool Set

Intentionally minimal — expose the most useful read and write operations.
Dangerous operations excluded or constrained.

### Read-only tools

| Tool | Maps to action | Key params |
|------|----------------|------------|
| `list_tabs` | `list` | windowId?, groupTitle?, limit? |
| `list_groups` | `group-list` | windowId?, limit? |
| `analyze_tabs` | `analyze` | windowId?, staleDays?, limit? |
| `inspect_tab` | `inspect` | tabId (single), signal |
| `ping` | `ping` | — |
| `history` | `history` | limit? |

### Write tools (with constraints)

| Tool | Maps to action | Constraints |
|------|----------------|-------------|
| `open_tabs` | `open` | urls required, max 10 |
| `focus_tab` | `focus` | single tabId |
| `close_tabs` | `close` | explicit tabIds only, max 10, confirmed auto |
| `group_update` | `group-update` | explicit group target |
| `group_assign` | `group-assign` | explicit tab + group |
| `undo` | `undo` | txid or latest |

### Excluded tools (too risky or too large for context)

- `screenshot` — returns binary data, agents should use browser tools
- `archive --all` — too destructive
- `close --apply` — blocked by design
- `report` — too verbose for context
- `dedupe` — complex multi-step, better as CLI

---

## 6. Implementation Plan

### Phase 1: Minimal viable MCP server

1. Add `@modelcontextprotocol/sdk` and `zod` as dependencies
2. Create `src/cli/mcp.ts` — MCP server entry point
3. Create `src/cli/lib/mcp-tools.ts` — tool definitions
4. Add `mcp` command to CLI router (`tabctl mcp`)
5. Add unit tests for tool registration and response formatting
6. Document MCP setup in README

### File structure

```
src/cli/
  mcp.ts                  # Entry point: tabctl mcp
  lib/
    mcp-tools.ts          # Tool definitions + response formatters
```

### Configuration (client side)

Agents configure tabctl MCP in their settings:

```json
{
  "mcpServers": {
    "tabctl": {
      "command": "tabctl",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

Or with a specific profile:

```json
{
  "mcpServers": {
    "tabctl": {
      "command": "tabctl",
      "args": ["mcp", "--profile", "edge"]
    }
  }
}
```

---

## 7. Proof of Concept

See `src/cli/mcp.ts` and `src/cli/lib/mcp-tools.ts` for a working
implementation that demonstrates:

- Tool registration with Zod schemas
- CLI-through architecture using `sendRequest()`
- Compact text response formatting
- Context-size-aware output truncation
- Error handling without process.exit

See `src/tests/unit/mcp.test.ts` for unit tests covering:

- Tool registration and listing
- Response formatting and truncation
- Schema validation
- Error response formatting
