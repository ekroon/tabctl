"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.configHome = configHome;
exports.configPath = configPath;
exports.stateHome = stateHome;
exports.loadConfig = loadConfig;
exports.resolveBrowser = resolveBrowser;
exports.resolveSocketPath = resolveSocketPath;
const fs_1 = __importDefault(require("fs"));
const os_1 = __importDefault(require("os"));
const path_1 = __importDefault(require("path"));
function configHome() {
    return process.env.XDG_CONFIG_HOME || path_1.default.join(os_1.default.homedir(), ".config");
}
function configPath() {
    return path_1.default.join(configHome(), "tabctl", "config.json");
}
function stateHome() {
    return process.env.XDG_STATE_HOME || path_1.default.join(os_1.default.homedir(), ".local", "state");
}
function loadConfig() {
    const resolvedPath = configPath();
    if (!fs_1.default.existsSync(resolvedPath)) {
        return null;
    }
    try {
        const raw = fs_1.default.readFileSync(resolvedPath, "utf8");
        const parsed = JSON.parse(raw);
        if (parsed === null || typeof parsed !== "object") {
            return null;
        }
        const browser = parsed.browser;
        if (browser === "edge" || browser === "chrome") {
            return { browser };
        }
        return {};
    }
    catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        process.stderr.write(`[tabctl] Failed to read config at ${resolvedPath}: ${message}\n`);
        return null;
    }
}
function resolveBrowser(config) {
    return config?.browser === "chrome" ? "chrome" : "edge";
}
function resolveSocketPath(stateHome, browser) {
    const socketName = browser === "chrome" ? "tabctl-chrome.sock" : "tabctl.sock";
    return path_1.default.join(stateHome, "tabctl", socketName);
}
