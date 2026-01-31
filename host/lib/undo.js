"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.appendUndoRecord = appendUndoRecord;
exports.readUndoRecords = readUndoRecords;
exports.filterByRetention = filterByRetention;
exports.findUndoRecord = findUndoRecord;
const fs_1 = __importDefault(require("fs"));
const path_1 = __importDefault(require("path"));
const DEFAULT_RETENTION_DAYS = 30;
function appendUndoRecord(filePath, record) {
    const dir = path_1.default.dirname(filePath);
    fs_1.default.mkdirSync(dir, { recursive: true, mode: 0o700 });
    fs_1.default.appendFileSync(filePath, `${JSON.stringify(record)}\n`, "utf8");
}
function readUndoRecords(filePath) {
    try {
        const content = fs_1.default.readFileSync(filePath, "utf8");
        const lines = content.split("\n").filter(Boolean);
        const records = [];
        for (const line of lines) {
            try {
                records.push(JSON.parse(line));
            }
            catch {
                // ignore malformed lines
            }
        }
        return records;
    }
    catch {
        return [];
    }
}
function filterByRetention(records, retentionDays = DEFAULT_RETENTION_DAYS, now = Date.now()) {
    const cutoff = now - retentionDays * 24 * 60 * 60 * 1000;
    return records.filter((record) => {
        const createdAt = record.createdAt;
        return !createdAt || createdAt >= cutoff;
    });
}
function findUndoRecord(filePath, txid, retentionDays = DEFAULT_RETENTION_DAYS, now = Date.now()) {
    const records = filterByRetention(readUndoRecords(filePath), retentionDays, now);
    for (let i = records.length - 1; i >= 0; i -= 1) {
        if (records[i].txid === txid) {
            return records[i];
        }
    }
    return null;
}
