#!/usr/bin/env node
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const fs_1 = __importDefault(require("fs"));
const net_1 = __importDefault(require("net"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const report_1 = require("./lib/report");
const SOCKET_PATH = process.env.TABARCHIVE_SOCKET || path_1.default.join(os_1.default.homedir(), ".tabarchive", "tabarchive.sock");
function createId() {
    return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}
function parseArgs(argv) {
    const args = [...argv];
    const command = args.shift();
    const options = { _: [] };
    while (args.length > 0) {
        const arg = args.shift();
        if (!arg.startsWith("--")) {
            options._.push(arg);
            continue;
        }
        const key = arg.slice(2);
        if (["all", "pretty", "confirm", "dry-run"].includes(key)) {
            options[key] = true;
            continue;
        }
        const value = args.shift();
        if (key === "tab") {
            if (!options.tab) {
                options.tab = [];
            }
            options.tab.push(value);
            continue;
        }
        options[key] = value;
    }
    return { command, options };
}
function sendRequest(payload) {
    return new Promise((resolve, reject) => {
        const client = net_1.default.createConnection(SOCKET_PATH);
        let buffer = "";
        client.on("connect", () => {
            client.write(`${JSON.stringify(payload)}\n`);
        });
        client.on("data", (data) => {
            buffer += data;
            const index = buffer.indexOf("\n");
            if (index === -1) {
                return;
            }
            const line = buffer.slice(0, index).trim();
            buffer = buffer.slice(index + 1);
            if (!line) {
                return;
            }
            try {
                const response = JSON.parse(line);
                client.end();
                client.destroy();
                resolve(response);
            }
            catch (error) {
                client.end();
                client.destroy();
                reject(error);
            }
        });
        client.on("error", (error) => {
            reject(error);
        });
    });
}
function printJson(payload, pretty = true) {
    const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
    process.stdout.write(`${output}\n`);
}
function errorOut(message) {
    printJson({ ok: false, error: { message } });
    process.exit(1);
}
async function main() {
    let { command, options } = parseArgs(process.argv.slice(2));
    const prettyOutput = options.pretty !== false;
    if (!command || command === "help" || options.help) {
        printJson({
            ok: true,
            data: {
                commands: [
                    "list",
                    "analyze",
                    "archive",
                    "close",
                    "report",
                    "undo",
                    "history",
                    "ping",
                ],
            },
        }, prettyOutput);
        return;
    }
    if (command === "close" && options["dry-run"]) {
        command = "analyze";
    }
    let action = command;
    let params = {};
    switch (command) {
        case "list":
            action = "list";
            break;
        case "ping":
            action = "ping";
            break;
        case "analyze":
            action = "analyze";
            params = {
                staleDays: options["stale-days"] ? Number(options["stale-days"]) : undefined,
            };
            break;
        case "archive":
            action = "archive";
            params = {
                all: Boolean(options.all),
                windowId: options.window ? Number(options.window) : undefined,
                groupTitle: options.group,
                groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
                tabIds: options.tab ? options.tab.map(Number) : undefined,
            };
            break;
        case "close":
            action = "close";
            if (options.apply) {
                params = { mode: "apply", analysisId: options.apply };
            }
            else {
                if (!options.confirm) {
                    errorOut("Direct close requires --confirm");
                }
                params = {
                    mode: "direct",
                    confirmed: true,
                    windowId: options.window ? Number(options.window) : undefined,
                    groupTitle: options.group,
                    groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
                    tabIds: options.tab ? options.tab.map(Number) : undefined,
                };
            }
            break;
        case "report":
            action = "report";
            params = {
                all: Boolean(options.all),
                windowId: options.window ? Number(options.window) : undefined,
                groupTitle: options.group,
                groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
                tabIds: options.tab ? options.tab.map(Number) : undefined,
            };
            break;
        case "undo":
            action = "undo";
            params = { txid: options._[0] };
            break;
        case "history":
            action = "history";
            params = { limit: options.limit ? Number(options.limit) : undefined };
            break;
        default:
            errorOut(`Unknown command: ${command}`);
    }
    const request = { id: createId(), action, params };
    let response;
    try {
        response = await sendRequest(request);
    }
    catch (error) {
        const message = error instanceof Error ? error.message : "Unknown error";
        errorOut(`Failed to connect to host: ${message}`);
        return;
    }
    if (!response.ok) {
        printJson(response, prettyOutput);
        process.exit(1);
    }
    if (command === "report") {
        const format = options.format || "json";
        const data = response.data;
        const entries = data?.entries || [];
        const generatedAt = data?.generatedAt;
        let content = "";
        if (format === "json") {
            content = JSON.stringify({ generatedAt, entries }, null, 2);
        }
        else if (format === "csv") {
            content = (0, report_1.renderCsv)(entries);
        }
        else if (format === "md") {
            content = (0, report_1.renderMarkdown)(entries, generatedAt);
        }
        else {
            errorOut(`Unknown report format: ${format}`);
        }
        if (options.out) {
            fs_1.default.writeFileSync(String(options.out), content, "utf8");
            printJson({ ok: true, data: { writtenTo: options.out, format, count: entries.length } }, prettyOutput);
            return;
        }
        if (format === "json") {
            printJson({ ok: true, data: { format, entries } }, prettyOutput);
            return;
        }
        printJson({ ok: true, data: { format, entries, content } }, prettyOutput);
        return;
    }
    printJson(response, prettyOutput);
}
main().catch((error) => {
    errorOut(error.message || "Unknown error");
});
