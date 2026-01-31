"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.createSocketPath = createSocketPath;
exports.startMockSocket = startMockSocket;
exports.stopMockSocket = stopMockSocket;
const fs_1 = __importDefault(require("fs"));
const net_1 = __importDefault(require("net"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
const crypto_1 = __importDefault(require("crypto"));
function createSocketPath() {
    const name = `ta-${process.pid}-${Date.now()}-${crypto_1.default.randomBytes(3).toString("hex")}.sock`;
    return path_1.default.join(os_1.default.tmpdir(), name);
}
async function startMockSocket(handler) {
    const socketPath = createSocketPath();
    if (fs_1.default.existsSync(socketPath)) {
        fs_1.default.unlinkSync(socketPath);
    }
    const requests = [];
    const sockets = new Set();
    const server = net_1.default.createServer((socket) => {
        sockets.add(socket);
        socket.setEncoding("utf8");
        let buffer = "";
        socket.on("data", (data) => {
            buffer += data;
            let index;
            while ((index = buffer.indexOf("\n")) >= 0) {
                const line = buffer.slice(0, index).trim();
                buffer = buffer.slice(index + 1);
                if (!line) {
                    continue;
                }
                let request;
                try {
                    request = JSON.parse(line);
                }
                catch {
                    socket.write(JSON.stringify({ ok: false, error: { message: "Invalid JSON" } }) + "\n");
                    continue;
                }
                requests.push(request);
                const response = handler(request) || {
                    ok: true,
                    action: request.action,
                    requestId: request.id,
                    data: {},
                };
                socket.write(JSON.stringify(response) + "\n");
            }
        });
        socket.on("close", () => {
            sockets.delete(socket);
        });
    });
    await new Promise((resolve) => {
        server.listen(socketPath, () => resolve());
    });
    return { socketPath, server, requests, sockets };
}
async function stopMockSocket(server, socketPath, sockets) {
    if (sockets) {
        for (const socket of sockets) {
            socket.destroy();
        }
    }
    await new Promise((resolve) => {
        server.close(() => resolve());
    });
    if (fs_1.default.existsSync(socketPath)) {
        fs_1.default.unlinkSync(socketPath);
    }
}
