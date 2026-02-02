import { DEFAULT_PAGE_LIMIT } from "./constants";
import { errorOut } from "./output";
import type { Options, PaginationResult } from "./types";

export function parseNumberOption(options: Options, key: string): number | null {
  if (!Object.prototype.hasOwnProperty.call(options, key)) {
    return null;
  }
  const value = Number(options[key]);
  if (!Number.isFinite(value)) {
    errorOut(`Invalid --${key} value`);
  }
  return value;
}

export function buildNextCommand(
  command: string,
  scopeArgs: string[],
  offset: number,
  limit: number
): string {
  const parts = ["tabctl", command, ...scopeArgs, "--offset", String(offset), "--limit", String(limit)];
  return parts.join(" ");
}

export function resolvePagination(
  options: Options,
  total: number,
  command: string,
  scopeArgs: string[]
): PaginationResult {
  const noPage = options["no-page"] === true;
  if (noPage) {
    return { offset: 0, limit: total, page: null };
  }
  
  const limitRaw = parseNumberOption(options, "limit");
  const offsetRaw = parseNumberOption(options, "offset");
  const limit = limitRaw != null ? Math.floor(limitRaw) : DEFAULT_PAGE_LIMIT;
  const offset = offsetRaw != null ? Math.floor(offsetRaw) : 0;
  
  if (!Number.isFinite(limit) || limit <= 0) {
    errorOut("--limit must be a positive number");
  }
  if (!Number.isFinite(offset) || offset < 0) {
    errorOut("--offset must be a non-negative number");
  }
  
  const remaining = total - offset;
  const returned = remaining > 0 ? Math.min(limit, remaining) : 0;
  const hasMore = offset + limit < total;
  const nextOffset = hasMore ? offset + limit : null;
  const hint = hasMore ? `Partial results. Next: ${buildNextCommand(command, scopeArgs, nextOffset as number, limit)}` : null;
  
  return {
    offset,
    limit,
    page: {
      offset,
      limit,
      returned,
      total,
      hasMore,
      nextOffset,
      hint,
    },
  };
}
