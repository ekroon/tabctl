"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const strict_1 = __importDefault(require("node:assert/strict"));
const node_test_1 = __importDefault(require("node:test"));
const report_1 = require("../../cli/lib/report");
(0, node_test_1.default)("renderCsv outputs headers and escapes values", () => {
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
    const csv = (0, report_1.renderCsv)(entries);
    const lines = csv.split("\n");
    strict_1.default.equal(lines[0], "windowLabel,groupTitle,title,url,description,lastFocusedAt");
    strict_1.default.match(lines[1], /"Title, with comma"/);
    strict_1.default.match(lines[1], /"A ""quoted"" description"/);
});
(0, node_test_1.default)("renderMarkdown groups by window and group", () => {
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
    const md = (0, report_1.renderMarkdown)(entries, 1700000000000);
    strict_1.default.match(md, /# Tab Report/);
    strict_1.default.match(md, /## W1/);
    strict_1.default.match(md, /### Alpha/);
    strict_1.default.match(md, /### Beta/);
    strict_1.default.match(md, /\[Example\]\(https:\/\/example\.com\)/);
});
