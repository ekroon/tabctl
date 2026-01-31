import assert from "node:assert/strict";
import test from "node:test";
import { renderCsv, renderMarkdown } from "../../cli/lib/report";

test("renderCsv outputs headers and escapes values", () => {
  const entries = [
    {
      windowLabel: "W1",
      groupTitle: "Research",
      title: "Title, with comma",
      url: "https://example.com",
      description: "A \"quoted\" description",
      lastFocusedAt: 123,
    },
  ];

  const csv = renderCsv(entries);
  const lines = csv.split("\n");
  assert.equal(lines[0], "windowLabel,groupTitle,title,url,description,lastFocusedAt");
  assert.match(lines[1], /"Title, with comma"/);
  assert.match(lines[1], /"A ""quoted"" description"/);
});

test("renderMarkdown groups by window and group", () => {
  const entries = [
    {
      windowId: 1,
      windowLabel: "W1",
      groupTitle: "Alpha",
      title: "Example",
      url: "https://example.com",
      description: "Desc",
    },
    {
      windowId: 1,
      windowLabel: "W1",
      groupTitle: "Beta",
      title: "Other",
      url: "https://example.org",
    },
  ];

  const md = renderMarkdown(entries, 1700000000000);
  assert.match(md, /# Tab Report/);
  assert.match(md, /## W1/);
  assert.match(md, /### Alpha/);
  assert.match(md, /### Beta/);
  assert.match(md, /\[Example\]\(https:\/\/example\.com\)/);
});
