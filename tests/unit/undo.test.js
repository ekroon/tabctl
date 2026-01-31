"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const strict_1 = __importDefault(require("node:assert/strict"));
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const node_test_1 = __importDefault(require("node:test"));
const undo_1 = require("../../host/lib/undo");
(0, node_test_1.default)("appendUndoRecord and readUndoRecords roundtrip", () => {
    const dir = fs_1.default.mkdtempSync(path_1.default.join(os_1.default.tmpdir(), "tabarchive-"));
    const filePath = path_1.default.join(dir, "undo.jsonl");
    const record = { txid: "tx-1", createdAt: Date.now(), action: "close" };
    (0, undo_1.appendUndoRecord)(filePath, record);
    const records = (0, undo_1.readUndoRecords)(filePath);
    strict_1.default.equal(records.length, 1);
    strict_1.default.equal(records[0].txid, "tx-1");
});
(0, node_test_1.default)("filterByRetention removes old records", () => {
    const now = Date.now();
    const records = [
        { txid: "old", createdAt: now - 40 * 24 * 60 * 60 * 1000 },
        { txid: "new", createdAt: now - 5 * 24 * 60 * 60 * 1000 },
    ];
    const filtered = (0, undo_1.filterByRetention)(records, 30, now);
    strict_1.default.equal(filtered.length, 1);
    strict_1.default.equal(filtered[0].txid, "new");
});
(0, node_test_1.default)("findUndoRecord respects retention", () => {
    const dir = fs_1.default.mkdtempSync(path_1.default.join(os_1.default.tmpdir(), "tabarchive-"));
    const filePath = path_1.default.join(dir, "undo.jsonl");
    const now = Date.now();
    (0, undo_1.appendUndoRecord)(filePath, { txid: "old", createdAt: now - 40 * 24 * 60 * 60 * 1000 });
    (0, undo_1.appendUndoRecord)(filePath, { txid: "new", createdAt: now - 5 * 24 * 60 * 60 * 1000 });
    const found = (0, undo_1.findUndoRecord)(filePath, "new", 30, now);
    const missing = (0, undo_1.findUndoRecord)(filePath, "old", 30, now);
    strict_1.default.ok(found);
    strict_1.default.equal(found?.txid, "new");
    strict_1.default.equal(missing, null);
});
