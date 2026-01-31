#!/usr/bin/env node
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import { renderCsv, renderMarkdown } from "./lib/report";

const SOCKET_PATH = process.env.TABARCHIVE_SOCKET || path.join(os.homedir(), ".tabarchive", "tabarchive.sock");

type Options = {
  _: string[];
  [key: string]: unknown;
};

function createId() {
  return `req-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function parseArgs(argv: string[]) {
  const args = [...argv];
  const command = args.shift();
  const options: Options = { _: [] };

  while (args.length > 0) {
    const arg = args.shift() as string;
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
      (options.tab as string[]).push(value as string);
      continue;
    }
    options[key] = value;
  }

  return { command, options };
}

function sendRequest(payload: Record<string, unknown>) {
  return new Promise<Record<string, unknown>>((resolve, reject) => {
    const client = net.createConnection(SOCKET_PATH);
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
      } catch (error) {
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

function printJson(payload: Record<string, unknown>, pretty = true) {
  const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
  process.stdout.write(`${output}\n`);
}

function errorOut(message: string) {
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
  let params: Record<string, unknown> = {};

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
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
      };
      break;
    case "close":
      action = "close";
      if (options.apply) {
        params = { mode: "apply", analysisId: options.apply };
      } else {
        if (!options.confirm) {
          errorOut("Direct close requires --confirm");
        }
        params = {
          mode: "direct",
          confirmed: true,
          windowId: options.window ? Number(options.window) : undefined,
          groupTitle: options.group,
          groupId: options["group-id"] ? Number(options["group-id"]) : undefined,
          tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
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
        tabIds: options.tab ? (options.tab as string[]).map(Number) : undefined,
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

  let response: Record<string, unknown>;
  try {
    response = await sendRequest(request);
  } catch (error) {
    const message = error instanceof Error ? error.message : "Unknown error";
    errorOut(`Failed to connect to host: ${message}`);
    return;
  }

  if (!response.ok) {
    printJson(response, prettyOutput);
    process.exit(1);
  }

  if (command === "report") {
    const format = (options.format as string) || "json";
    const data = response.data as { entries?: Array<Record<string, unknown>>; generatedAt?: number } | undefined;
    const entries = data?.entries || [];
    const generatedAt = data?.generatedAt;
    let content = "";

    if (format === "json") {
      content = JSON.stringify({ generatedAt, entries }, null, 2);
    } else if (format === "csv") {
      content = renderCsv(entries);
    } else if (format === "md") {
      content = renderMarkdown(entries, generatedAt);
    } else {
      errorOut(`Unknown report format: ${format}`);
    }

    if (options.out) {
      fs.writeFileSync(String(options.out), content, "utf8");
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

main().catch((error: Error) => {
  errorOut(error.message || "Unknown error");
});
