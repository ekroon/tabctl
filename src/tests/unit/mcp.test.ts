import assert from "node:assert/strict";
import test from "node:test";
import { runCli, parseOutput } from "./cli-helpers";

// Import the formatters directly for unit testing
import {
  formatListResponse,
  formatGroupListResponse,
  formatAnalyzeResponse,
  formatInspectResponse,
  formatSimpleResponse,
  truncate,
} from "../../cli/lib/mcp-tools";

// ============================================================================
// Truncation
// ============================================================================

test("truncate returns short text unchanged", () => {
  assert.equal(truncate("hello", 100), "hello");
});

test("truncate truncates long text", () => {
  const long = "x".repeat(200);
  const result = truncate(long, 100);
  assert.ok(result.length < 200, "truncated output should be shorter than input");
  assert.match(result, /… \(output truncated\)/);
});

// ============================================================================
// formatListResponse
// ============================================================================

test("formatListResponse formats tabs from windows", () => {
  const response = {
    ok: true,
    data: {
      windows: [
        {
          id: 1,
          tabs: [
            { id: 10, title: "Example", url: "https://example.com" },
            { id: 11, title: "GitHub", url: "https://github.com" },
          ],
        },
      ],
    },
  };
  const result = formatListResponse(response, 20);
  assert.match(result, /Window 1 \(2 tabs\)/);
  assert.match(result, /\[Tab 10\] Example/);
  assert.match(result, /\[Tab 11\] GitHub/);
});

test("formatListResponse respects limit", () => {
  const tabs = Array.from({ length: 10 }, (_, i) => ({
    id: i,
    title: `Tab ${i}`,
    url: `https://example.com/${i}`,
  }));
  const response = { ok: true, data: { windows: [{ id: 1, tabs }] } };
  const result = formatListResponse(response, 3);
  // Should contain tabs 0, 1, 2 but not tab 3
  assert.match(result, /\[Tab 0\]/);
  assert.match(result, /\[Tab 2\]/);
  assert.ok(!result.includes("[Tab 3]"));
});

test("formatListResponse handles empty windows", () => {
  const response = { ok: true, data: { windows: [] } };
  const result = formatListResponse(response, 20);
  assert.match(result, /No tabs found/);
});

test("formatListResponse handles error response", () => {
  const response = { ok: false, error: { message: "Connection failed" } };
  const result = formatListResponse(response, 20);
  assert.match(result, /Error: Connection failed/);
});

test("formatListResponse includes pagination hint", () => {
  const response = {
    ok: true,
    data: {
      windows: [{ id: 1, tabs: [{ id: 1, title: "T", url: "u" }] }],
      page: { hasMore: true, nextOffset: 20 },
    },
  };
  const result = formatListResponse(response, 20);
  assert.match(result, /More results available/);
});

// ============================================================================
// formatGroupListResponse
// ============================================================================

test("formatGroupListResponse formats groups", () => {
  const response = {
    ok: true,
    data: {
      groups: [
        { id: 1, title: "Work", tabCount: 5, color: "blue", windowId: 100 },
        { id: 2, title: "Personal", tabCount: 3, color: "green", windowId: 100 },
      ],
    },
  };
  const result = formatGroupListResponse(response, 20);
  assert.match(result, /\[Group 1\] Work \(blue\)/);
  assert.match(result, /5 tabs/);
  assert.match(result, /\[Group 2\] Personal/);
});

test("formatGroupListResponse handles empty groups", () => {
  const response = { ok: true, data: { groups: [] } };
  const result = formatGroupListResponse(response, 20);
  assert.match(result, /No groups found/);
});

// ============================================================================
// formatAnalyzeResponse
// ============================================================================

test("formatAnalyzeResponse includes summary and candidates", () => {
  const response = {
    ok: true,
    data: {
      summary: { totalTabs: 20, duplicates: 3, stale: 2 },
      candidates: [
        { id: 5, title: "Dup Page", url: "https://dup.com", reasons: [{ type: "duplicate" }] },
        { id: 6, title: "Old Page", url: "https://old.com", reasons: [{ type: "stale" }] },
      ],
    },
  };
  const result = formatAnalyzeResponse(response, 20);
  assert.match(result, /Analysis Summary/);
  assert.match(result, /totalTabs: 20/);
  assert.match(result, /Top 2 candidates/);
  assert.match(result, /duplicate/);
  assert.match(result, /stale/);
});

test("formatAnalyzeResponse handles no candidates", () => {
  const response = {
    ok: true,
    data: { summary: { totalTabs: 5 }, candidates: [] },
  };
  const result = formatAnalyzeResponse(response, 20);
  assert.match(result, /No candidates found/);
});

// ============================================================================
// formatInspectResponse
// ============================================================================

test("formatInspectResponse formats entries", () => {
  const response = {
    ok: true,
    data: {
      entries: [
        {
          tab: { id: 1, title: "Example", url: "https://example.com" },
          signals: { "page-meta": { description: "An example page", h1: "Hello" } },
        },
      ],
    },
  };
  const result = formatInspectResponse(response);
  assert.match(result, /\[Tab 1\] Example/);
  assert.match(result, /page-meta/);
  assert.match(result, /An example page/);
});

test("formatInspectResponse handles empty entries", () => {
  const response = { ok: true, data: { entries: [] } };
  const result = formatInspectResponse(response);
  assert.match(result, /No inspection results/);
});

// ============================================================================
// formatSimpleResponse
// ============================================================================

test("formatSimpleResponse shows success", () => {
  const response = { ok: true, data: {} };
  const result = formatSimpleResponse(response, "ping");
  assert.match(result, /ping succeeded/);
});

test("formatSimpleResponse shows summary", () => {
  const response = { ok: true, data: { summary: { closedTabs: 3 } } };
  const result = formatSimpleResponse(response, "close");
  assert.match(result, /close succeeded/);
  assert.match(result, /closedTabs: 3/);
});

test("formatSimpleResponse shows txid for undo", () => {
  const response = { ok: true, data: { txid: "tx-123" } };
  const result = formatSimpleResponse(response, "close");
  assert.match(result, /txid: tx-123/);
  assert.match(result, /undo tool/);
});

test("formatSimpleResponse shows error", () => {
  const response = { ok: false, error: { message: "Tab not found", hint: "Check tab ID" } };
  const result = formatSimpleResponse(response, "focus");
  assert.match(result, /Error: Tab not found/);
  assert.match(result, /Hint: Check tab ID/);
});

// ============================================================================
// CLI integration — help includes mcp command
// ============================================================================

test("help --json includes mcp command", async () => {
  const result = await runCli(["help", "--json"]);
  assert.equal(result.status, 0);
  const output = parseOutput(result);
  const mcpCmd = output.data.commands.find((c: { name: string }) => c.name === "mcp");
  assert.ok(mcpCmd, "mcp command should appear in help");
  assert.match(mcpCmd.description, /MCP/i);
});

test("help text includes mcp command", async () => {
  const result = await runCli(["help"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /mcp/);
});
