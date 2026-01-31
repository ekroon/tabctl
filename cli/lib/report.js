"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.renderCsv = renderCsv;
exports.renderMarkdown = renderMarkdown;
function renderCsv(entries) {
    const header = ["windowLabel", "groupTitle", "title", "url", "description", "lastFocusedAt"];
    const lines = [header.join(",")];
    const csvEscape = (value) => {
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
function renderMarkdown(entries, generatedAt) {
    const lines = [];
    const date = generatedAt ? new Date(generatedAt).toISOString() : new Date().toISOString();
    lines.push(`# Tab Report (${date})`);
    const grouped = new Map();
    for (const entry of entries) {
        const windowKey = entry.windowLabel || `W${entry.windowId}`;
        const groupKey = entry.groupTitle || "Ungrouped";
        if (!grouped.has(windowKey)) {
            grouped.set(windowKey, new Map());
        }
        const windowMap = grouped.get(windowKey);
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
                const title = item.title || item.url || "(untitled)";
                const url = item.url;
                const link = url ? `[${title}](${url})` : title;
                const desc = item.description ? ` - ${item.description}` : "";
                lines.push(`- ${link}${desc}`);
            }
        }
    }
    return lines.join("\n");
}
