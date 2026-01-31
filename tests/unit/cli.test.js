"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const strict_1 = __importDefault(require("node:assert/strict"));
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const node_child_process_1 = require("node:child_process");
const node_test_1 = __importDefault(require("node:test"));
const socket_1 = require("./socket");
const cliPath = path_1.default.resolve(__dirname, "../../cli/tabctl.js");
async function runCli(args, socketPath, extraEnv) {
    const env = { ...process.env };
    if (socketPath) {
        env.TABARCHIVE_SOCKET = socketPath;
    }
    if (extraEnv) {
        Object.assign(env, extraEnv);
    }
    return new Promise((resolve, reject) => {
        const child = (0, node_child_process_1.spawn)(process.execPath, [cliPath, ...args], { env });
        let stdout = "";
        let stderr = "";
        const timeout = setTimeout(() => {
            child.kill("SIGKILL");
            reject(new Error("CLI timeout"));
        }, 2000);
        child.stdout.on("data", (data) => {
            stdout += data.toString();
        });
        child.stderr.on("data", (data) => {
            stderr += data.toString();
        });
        child.on("error", (error) => {
            clearTimeout(timeout);
            reject(error);
        });
        child.on("exit", (code) => {
            clearTimeout(timeout);
            resolve({ status: code, stdout, stderr });
        });
    });
}
(0, node_test_1.default)("list sends list action", async () => {
    const { socketPath, server, requests, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: { value: "ok" },
    }));
    const result = await runCli(["list"], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, true);
    strict_1.default.equal(requests.length, 1);
    strict_1.default.equal(requests[0].action, "list");
});
(0, node_test_1.default)("close without confirm fails", async () => {
    const result = await runCli(["close", "--tab", "123"]);
    strict_1.default.equal(result.status, 1);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, false);
    strict_1.default.equal(output.error.message, "Direct close requires --confirm");
});
(0, node_test_1.default)("close --dry-run maps to analyze", async () => {
    const { socketPath, server, requests, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
    }));
    const result = await runCli(["close", "--dry-run"], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    strict_1.default.equal(requests[0].action, "analyze");
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, true);
});
(0, node_test_1.default)("analyze passes tab ids and github options", async () => {
    const { socketPath, server, requests, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: { candidates: [], totals: { tabs: 0, candidates: 0 } },
    }));
    const result = await runCli([
        "analyze",
        "--tab",
        "12",
        "--github",
        "--github-concurrency",
        "3",
        "--github-timeout-ms",
        "2500",
        "--progress",
    ], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    strict_1.default.equal(requests[0].action, "analyze");
    const params = requests[0].params;
    strict_1.default.deepEqual(params?.tabIds, [12]);
    strict_1.default.equal(params?.checkGitHub, true);
    strict_1.default.equal(params?.githubConcurrency, 3);
    strict_1.default.equal(params?.githubTimeoutMs, 2500);
    strict_1.default.equal(params?.progress, true);
});
(0, node_test_1.default)("report format md returns markdown content", async () => {
    const { socketPath, server, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: {
            generatedAt: 1700000000000,
            entries: [
                {
                    windowId: 1,
                    windowLabel: "W1",
                    groupTitle: "Test",
                    title: "Example",
                    url: "https://example.com",
                    description: "Desc",
                },
            ],
        },
    }));
    const result = await runCli(["report", "--format", "md"], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, true);
    strict_1.default.equal(output.data.format, "md");
    strict_1.default.match(output.data.content, /# Tab Report/);
});
(0, node_test_1.default)("inspect passes signal options", async () => {
    const { socketPath, server, requests, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: { entries: [] },
    }));
    const result = await runCli([
        "inspect",
        "--tab",
        "42",
        "--signal",
        "page-meta",
        "--signal",
        "github-state",
        "--signal",
        "selector",
        "--selector",
        "price=.price",
        "--signal-concurrency",
        "2",
        "--signal-timeout-ms",
        "1500",
        "--progress",
    ], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    strict_1.default.equal(requests[0].action, "inspect");
    const params = requests[0].params;
    strict_1.default.deepEqual(params?.tabIds, [42]);
    strict_1.default.deepEqual(params?.signals, ["page-meta", "github-state", "selector"]);
    strict_1.default.deepEqual(params?.selectorSpecs, [{ name: "price", selector: ".price" }]);
    strict_1.default.equal(params?.signalConcurrency, 2);
    strict_1.default.equal(params?.signalTimeoutMs, 1500);
    strict_1.default.equal(params?.progress, true);
});
(0, node_test_1.default)("focus passes tab id", async () => {
    const { socketPath, server, requests, sockets } = await (0, socket_1.startMockSocket)((req) => ({
        ok: true,
        action: req.action,
        requestId: req.id,
        data: { tabId: 99, windowId: 1 },
    }));
    const result = await runCli(["focus", "--tab", "99"], socketPath);
    await (0, socket_1.stopMockSocket)(server, socketPath, sockets);
    strict_1.default.equal(result.status, 0);
    strict_1.default.equal(requests[0].action, "focus");
    const params = requests[0].params;
    strict_1.default.equal(params?.tabId, 99);
});
(0, node_test_1.default)("policy init creates default file", async () => {
    const dir = fs_1.default.mkdtempSync(path_1.default.join(os_1.default.tmpdir(), "tabarchive-policy-init-"));
    const result = await runCli(["policy", "--init"], undefined, { XDG_CONFIG_HOME: dir });
    strict_1.default.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, true);
    const policyPath = path_1.default.join(dir, "tabctl", "policy.json");
    strict_1.default.ok(fs_1.default.existsSync(policyPath));
    const raw = fs_1.default.readFileSync(policyPath, "utf8");
    strict_1.default.match(raw, /"pinned"/);
});
(0, node_test_1.default)("help outputs plain text by default", async () => {
    const result = await runCli(["help"]);
    strict_1.default.equal(result.status, 0);
    strict_1.default.match(result.stdout, /tabctl - Edge tab management CLI/);
});
(0, node_test_1.default)("help supports json output", async () => {
    const result = await runCli(["help", "--json"]);
    strict_1.default.equal(result.status, 0);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, true);
    strict_1.default.ok(output.data?.commands);
});
(0, node_test_1.default)("no-policy flag is rejected", async () => {
    const result = await runCli(["list", "--no-policy"]);
    strict_1.default.equal(result.status, 1);
    const output = JSON.parse(result.stdout.trim());
    strict_1.default.equal(output.ok, false);
    strict_1.default.match(output.error.message, /no-policy/);
});
