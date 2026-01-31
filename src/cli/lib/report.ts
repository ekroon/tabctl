export function renderCsv(entries: Array<Record<string, unknown>>): string {
  const header = ["windowLabel", "groupTitle", "title", "url", "description", "lastFocusedAt"];
  const lines = [header.join(",")];

  const csvEscape = (value: unknown): string => {
    if (value == null) {
      return "";
    }
    const str = String(value);
    if (/[",\n]/.test(str)) {
      return `"${str.replace(/"/g, '""')}"`;
    }
    return str;
  };

  for (const entry of entries) {
    lines.push([
      csvEscape(entry.windowLabel),
      csvEscape(entry.groupTitle || ""),
      csvEscape(entry.title || ""),
      csvEscape(entry.url || ""),
      csvEscape(entry.description || ""),
      csvEscape(entry.lastFocusedAt || ""),
    ].join(","));
  }

  return lines.join("\n");
}

export function renderMarkdown(entries: Array<Record<string, unknown>>, generatedAt?: number): string {
  const lines: string[] = [];
  const date = generatedAt ? new Date(generatedAt).toISOString() : new Date().toISOString();
  lines.push(`# Tab Report (${date})`);

  const grouped = new Map<string, Map<string, Array<Record<string, unknown>>>>();
  for (const entry of entries) {
    const windowKey = (entry.windowLabel as string) || `W${entry.windowId}`;
    const groupKey = (entry.groupTitle as string) || "Ungrouped";
    if (!grouped.has(windowKey)) {
      grouped.set(windowKey, new Map());
    }
    const windowMap = grouped.get(windowKey) as Map<string, Array<Record<string, unknown>>>;
    if (!windowMap.has(groupKey)) {
      windowMap.set(groupKey, []);
    }
    windowMap.get(groupKey)?.push(entry);
  }

  for (const [windowKey, groupMap] of grouped.entries()) {
    lines.push(`\n## ${windowKey}`);
    for (const [groupKey, items] of groupMap.entries()) {
      lines.push(`\n### ${groupKey}`);
      for (const item of items) {
        const title = (item.title as string) || (item.url as string) || "(untitled)";
        const url = item.url as string | undefined;
        const link = url ? `[${title}](${url})` : title;
        const desc = item.description ? ` - ${item.description}` : "";
        lines.push(`- ${link}${desc}`);
      }
    }
  }

  return lines.join("\n");
}
