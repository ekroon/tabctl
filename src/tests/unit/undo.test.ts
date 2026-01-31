import assert from "node:assert/strict";
import fs from "fs";
import os from "os";
import path from "path";
import test from "node:test";
import { appendUndoRecord, readUndoRecords, filterByRetention, findUndoRecord } from "../../host/lib/undo";

test("appendUndoRecord and readUndoRecords roundtrip", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabarchive-"));
  const filePath = path.join(dir, "undo.jsonl");
  const record = { txid: "tx-1", createdAt: Date.now(), action: "close" };

  appendUndoRecord(filePath, record);
  const records = readUndoRecords(filePath);

  assert.equal(records.length, 1);
  assert.equal(records[0].txid, "tx-1");
});

test("filterByRetention removes old records", () => {
  const now = Date.now();
  const records = [
    { txid: "old", createdAt: now - 40 * 24 * 60 * 60 * 1000 },
    { txid: "new", createdAt: now - 5 * 24 * 60 * 60 * 1000 },
  ];

  const filtered = filterByRetention(records, 30, now);
  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].txid, "new");
});

test("findUndoRecord respects retention", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tabarchive-"));
  const filePath = path.join(dir, "undo.jsonl");
  const now = Date.now();
  appendUndoRecord(filePath, { txid: "old", createdAt: now - 40 * 24 * 60 * 60 * 1000 });
  appendUndoRecord(filePath, { txid: "new", createdAt: now - 5 * 24 * 60 * 60 * 1000 });

  const found = findUndoRecord(filePath, "new", 30, now);
  const missing = findUndoRecord(filePath, "old", 30, now);

  assert.ok(found);
  assert.equal(found?.txid, "new");
  assert.equal(missing, null);
});
