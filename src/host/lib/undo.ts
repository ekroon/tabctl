import fs from "fs";
import path from "path";

const DEFAULT_RETENTION_DAYS = 30;

export function appendUndoRecord(filePath: string, record: Record<string, unknown>): void {
  const dir = path.dirname(filePath);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.appendFileSync(filePath, `${JSON.stringify(record)}\n`, "utf8");
}

export function readUndoRecords(filePath: string): Array<Record<string, unknown>> {
  try {
    const content = fs.readFileSync(filePath, "utf8");
    const lines = content.split("\n").filter(Boolean);
    const records: Array<Record<string, unknown>> = [];
    for (const line of lines) {
      try {
        records.push(JSON.parse(line));
      } catch {
        // ignore malformed lines
      }
    }
    return records;
  } catch {
    return [];
  }
}

export function filterByRetention(
  records: Array<Record<string, unknown>>,
  retentionDays = DEFAULT_RETENTION_DAYS,
  now = Date.now(),
): Array<Record<string, unknown>> {
  const cutoff = now - retentionDays * 24 * 60 * 60 * 1000;
  return records.filter((record) => {
    const createdAt = record.createdAt as number | undefined;
    return !createdAt || createdAt >= cutoff;
  });
}

export function findUndoRecord(
  filePath: string,
  txid: string,
  retentionDays = DEFAULT_RETENTION_DAYS,
  now = Date.now(),
): Record<string, unknown> | null {
  const records = filterByRetention(readUndoRecords(filePath), retentionDays, now);
  for (let i = records.length - 1; i >= 0; i -= 1) {
    if (records[i].txid === txid) {
      return records[i];
    }
  }
  return null;
}

export function findLatestUndoRecord(
  filePath: string,
  retentionDays = DEFAULT_RETENTION_DAYS,
  now = Date.now(),
): Record<string, unknown> | null {
  const records = filterByRetention(readUndoRecords(filePath), retentionDays, now);
  if (!records.length) {
    return null;
  }
  return records[records.length - 1] || null;
}
