const HOST_NAME = "com.example.tabarchive";
const KEEPALIVE_ALARM = "tabarchive-keepalive";
const KEEPALIVE_INTERVAL_MINUTES = 1;
const DEFAULT_STALE_DAYS = 30;
const DESCRIPTION_MAX_LENGTH = 250;
const SELECTOR_VALUE_MAX_LENGTH = 500;
const state = {
    port: null,
    archiveWindowId: null,
    archiveWindowIdLoaded: false,
    lastFocused: {},
    lastFocusedLoaded: false,
};
function log(...args) {
    console.log("[TabArchive]", ...args);
}
function sendResponse(id, ok, payload) {
    if (!state.port) {
        return;
    }
    if (ok) {
        state.port.postMessage({ id, ok: true, data: payload });
        return;
    }
    const error = payload instanceof Error
        ? { message: payload.message, stack: payload.stack }
        : payload;
    state.port.postMessage({ id, ok: false, error });
}
function connectNative() {
    if (state.port) {
        return;
    }
    try {
        const port = chrome.runtime.connectNative(HOST_NAME);
        state.port = port;
        port.onMessage.addListener(handleNativeMessage);
        port.onDisconnect.addListener(() => {
            const lastError = chrome.runtime.lastError;
            if (lastError) {
                log("Native host disconnected:", lastError.message);
            }
            else {
                log("Native host disconnected");
            }
            state.port = null;
        });
        log("Native host connected");
    }
    catch (error) {
        log("Native host connection failed", error);
    }
}
chrome.runtime.onInstalled.addListener(() => {
    connectNative();
    chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: KEEPALIVE_INTERVAL_MINUTES });
});
chrome.runtime.onStartup.addListener(() => {
    connectNative();
    chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: KEEPALIVE_INTERVAL_MINUTES });
});
chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === KEEPALIVE_ALARM) {
        connectNative();
    }
});
async function ensureLastFocusedLoaded() {
    if (state.lastFocusedLoaded) {
        return;
    }
    const stored = await chrome.storage.local.get("lastFocused");
    state.lastFocused = stored.lastFocused || {};
    state.lastFocusedLoaded = true;
}
async function ensureArchiveWindowIdLoaded() {
    if (state.archiveWindowIdLoaded) {
        return;
    }
    const stored = await chrome.storage.local.get("archiveWindowId");
    state.archiveWindowId = stored.archiveWindowId || null;
    state.archiveWindowIdLoaded = true;
}
async function setLastFocused(tabId) {
    await ensureLastFocusedLoaded();
    state.lastFocused[String(tabId)] = Date.now();
    await chrome.storage.local.set({ lastFocused: state.lastFocused });
}
chrome.tabs.onActivated.addListener((info) => {
    setLastFocused(info.tabId).catch((error) => log("Failed to set last focused", error));
});
chrome.tabs.onRemoved.addListener((tabId) => {
    ensureLastFocusedLoaded().then(() => {
        const key = String(tabId);
        if (state.lastFocused[key]) {
            delete state.lastFocused[key];
            chrome.storage.local.set({ lastFocused: state.lastFocused });
        }
    }).catch((error) => log("Failed to prune last focused", error));
});
chrome.windows.onFocusChanged.addListener((windowId) => {
    if (windowId === chrome.windows.WINDOW_ID_NONE) {
        return;
    }
    chrome.tabs.query({ windowId, active: true }).then((tabs) => {
        if (tabs[0] && tabs[0].id != null) {
            setLastFocused(tabs[0].id).catch((error) => log("Failed to set last focused", error));
        }
    }).catch((error) => log("Failed to query active tab", error));
});
async function handleNativeMessage(message) {
    if (!message || typeof message !== "object") {
        return;
    }
    const { id, action, params } = message;
    if (!id || !action) {
        return;
    }
    try {
        const data = await handleAction(action, params || {}, id);
        sendResponse(id, true, data);
    }
    catch (error) {
        sendResponse(id, false, error);
    }
}
function sendProgress(id, payload) {
    if (!state.port) {
        return;
    }
    state.port.postMessage({ id, progress: true, data: payload });
}
async function handleAction(action, params, requestId) {
    switch (action) {
        case "ping":
            return { now: Date.now() };
        case "list":
            return await getTabSnapshot();
        case "analyze":
            return await analyzeTabs(params, requestId);
        case "inspect":
            return await inspectTabs(params, requestId);
        case "focus":
            return await focusTab(params);
        case "archive":
            return await archiveTabs(params);
        case "close":
            return await closeTabs(params);
        case "report":
            return await reportTabs(params);
        case "undo":
            return await undoTransaction(params);
        default:
            throw new Error(`Unknown action: ${action}`);
    }
}
function isScriptableUrl(url) {
    return typeof url === "string" && (url.startsWith("http://") || url.startsWith("https://"));
}
function normalizeUrl(rawUrl) {
    if (!rawUrl || typeof rawUrl !== "string") {
        return null;
    }
    let url;
    try {
        url = new URL(rawUrl);
    }
    catch {
        return null;
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") {
        return null;
    }
    url.hash = "";
    const dropKeys = new Set([
        "fbclid",
        "gclid",
        "igshid",
        "mc_cid",
        "mc_eid",
        "ref",
        "ref_src",
        "ref_url",
        "utm_campaign",
        "utm_content",
        "utm_medium",
        "utm_source",
        "utm_term",
        "utm_name",
        "si",
    ]);
    for (const key of Array.from(url.searchParams.keys())) {
        if (key.startsWith("utm_") || dropKeys.has(key)) {
            url.searchParams.delete(key);
        }
    }
    const search = url.searchParams.toString();
    url.search = search ? `?${search}` : "";
    return url.toString();
}
async function getTabSnapshot() {
    await ensureLastFocusedLoaded();
    const windows = await chrome.windows.getAll({ populate: true, windowTypes: ["normal"] });
    const groups = await chrome.tabGroups.query({});
    const groupById = new Map(groups.map((group) => [group.id, group]));
    const snapshot = windows.map((win) => {
        const tabs = (win.tabs || []).map((tab) => {
            const group = tab.groupId !== -1 ? groupById.get(tab.groupId) : null;
            return {
                tabId: tab.id,
                windowId: win.id,
                index: tab.index,
                url: tab.url,
                title: tab.title,
                active: tab.active,
                pinned: tab.pinned,
                groupId: tab.groupId,
                groupTitle: group ? group.title : null,
                groupColor: group ? group.color : null,
                lastFocusedAt: state.lastFocused[String(tab.id)] || null,
            };
        });
        const windowGroups = groups
            .filter((group) => group.windowId === win.id)
            .map((group) => ({
            groupId: group.id,
            title: group.title,
            color: group.color,
            collapsed: group.collapsed,
        }));
        return {
            windowId: win.id,
            focused: win.focused,
            state: win.state,
            tabs,
            groups: windowGroups,
        };
    });
    return {
        generatedAt: Date.now(),
        windows: snapshot,
    };
}
function flattenTabs(snapshot) {
    const result = [];
    for (const win of snapshot.windows) {
        for (const tab of win.tabs) {
            result.push(tab);
        }
    }
    return result;
}
function isGitHubIssueOrPr(url) {
    if (!url) {
        return false;
    }
    return /^https:\/\/github\.com\/[^/]+\/[^/]+\/(issues|pull)\/\d+/.test(url);
}
async function executeWithTimeout(tabId, timeoutMs, func, args = []) {
    const execPromise = chrome.scripting.executeScript({
        target: { tabId },
        func,
        args,
    });
    const timeoutPromise = new Promise((resolve) => {
        const handle = setTimeout(() => {
            clearTimeout(handle);
            resolve(null);
        }, timeoutMs);
    });
    try {
        const result = await Promise.race([execPromise, timeoutPromise]);
        if (!result || !Array.isArray(result)) {
            return null;
        }
        const [{ result: value }] = result;
        return value ?? null;
    }
    catch {
        return null;
    }
}
async function detectGitHubState(tabId, timeoutMs) {
    const result = await executeWithTimeout(tabId, timeoutMs, () => {
        const stateEl = document.querySelector(".gh-header-meta .State") ||
            document.querySelector(".State") ||
            document.querySelector(".js-issue-state");
        if (!stateEl) {
            return null;
        }
        const text = (stateEl.textContent || "").trim().toLowerCase();
        if (text.includes("merged")) {
            return "merged";
        }
        if (text.includes("closed")) {
            return "closed";
        }
        if (text.includes("open")) {
            return "open";
        }
        return null;
    });
    return typeof result === "string" ? result : null;
}
async function extractPageMeta(tabId, timeoutMs) {
    const result = await executeWithTimeout(tabId, timeoutMs, () => {
        const pickContent = (selector) => {
            const el = document.querySelector(selector);
            if (!el) {
                return "";
            }
            const content = el.getAttribute("content") || el.textContent || "";
            return content.trim();
        };
        const description = pickContent("meta[name='description']") ||
            pickContent("meta[property='og:description']") ||
            pickContent("meta[name='twitter:description']");
        const h1 = document.querySelector("h1");
        const h1Text = h1 ? h1.textContent?.trim() : "";
        return {
            description: description.replace(/\s+/g, " ").trim(),
            h1: (h1Text || "").replace(/\s+/g, " ").trim(),
        };
    });
    if (!result || typeof result !== "object") {
        return null;
    }
    const meta = result;
    return {
        description: (meta.description || "").slice(0, DESCRIPTION_MAX_LENGTH),
        h1: (meta.h1 || "").slice(0, DESCRIPTION_MAX_LENGTH),
    };
}
async function extractSelectorSignal(tabId, specs, timeoutMs) {
    if (!specs.length) {
        return null;
    }
    const result = await executeWithTimeout(tabId, timeoutMs, (rawSpecs, maxLen) => {
        const values = {};
        const missing = [];
        const errors = {};
        for (const raw of rawSpecs) {
            const selector = typeof raw.selector === "string" ? raw.selector : "";
            if (!selector) {
                continue;
            }
            const name = typeof raw.name === "string" && raw.name ? raw.name : selector;
            const attr = typeof raw.attr === "string" ? raw.attr : "text";
            const all = Boolean(raw.all);
            try {
                const elements = Array.from(document.querySelectorAll(selector));
                if (!elements.length) {
                    missing.push(name);
                    continue;
                }
                const getValue = (el) => {
                    let value = "";
                    if (attr === "text") {
                        value = el.textContent || "";
                    }
                    else {
                        value = el.getAttribute(attr) || "";
                    }
                    return value.replace(/\s+/g, " ").trim().slice(0, maxLen);
                };
                if (all) {
                    values[name] = elements.map(getValue).filter((val) => val.length > 0);
                }
                else {
                    values[name] = getValue(elements[0]);
                }
            }
            catch (err) {
                const message = err instanceof Error ? err.message : "selector_error";
                errors[name] = message;
            }
        }
        return { values, missing, errors };
    }, [specs, SELECTOR_VALUE_MAX_LENGTH]);
    if (!result || typeof result !== "object") {
        return null;
    }
    return result;
}
async function analyzeTabs(params, requestId) {
    const staleDays = Number.isFinite(params.staleDays) ? params.staleDays : DEFAULT_STALE_DAYS;
    const checkGitHub = params.checkGitHub === true;
    const requestedTabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : null;
    const githubConcurrencyRaw = Number(params.githubConcurrency);
    const githubConcurrency = Number.isFinite(githubConcurrencyRaw) && githubConcurrencyRaw > 0
        ? Math.min(10, Math.floor(githubConcurrencyRaw))
        : 4;
    const githubTimeoutRaw = Number(params.githubTimeoutMs);
    const githubTimeoutMs = Number.isFinite(githubTimeoutRaw) && githubTimeoutRaw > 0
        ? Math.floor(githubTimeoutRaw)
        : 4000;
    const progressEnabled = params.progress === true;
    const snapshot = await getTabSnapshot();
    const tabs = flattenTabs(snapshot);
    const selectedTabs = requestedTabIds
        ? tabs.filter((tab) => requestedTabIds.includes(tab.tabId))
        : tabs;
    const scopeTabs = requestedTabIds ? selectedTabs : tabs;
    const now = Date.now();
    const startedAt = Date.now();
    let githubChecked = 0;
    let githubTotal = 0;
    let githubMatched = 0;
    const normalizedMap = new Map();
    const duplicates = new Map();
    for (const tab of scopeTabs) {
        const normalized = normalizeUrl(tab.url);
        if (!normalized) {
            continue;
        }
        if (normalizedMap.has(normalized)) {
            const existing = normalizedMap.get(normalized);
            duplicates.set(tab.tabId, existing.tabId);
        }
        else {
            normalizedMap.set(normalized, tab);
        }
    }
    const candidateMap = new Map();
    const addReason = (tab, reason) => {
        const tabId = tab.tabId;
        const entry = candidateMap.get(tabId) || { tab, reasons: [] };
        entry.reasons.push(reason);
        candidateMap.set(tabId, entry);
    };
    for (const tab of selectedTabs) {
        if (duplicates.has(tab.tabId)) {
            addReason(tab, {
                type: "duplicate",
                detail: `Matches tab ${duplicates.get(tab.tabId)}`,
            });
        }
        if (tab.lastFocusedAt) {
            const ageDays = (now - tab.lastFocusedAt) / (24 * 60 * 60 * 1000);
            if (ageDays >= staleDays) {
                addReason(tab, {
                    type: "stale",
                    detail: `Last focused ${Math.floor(ageDays)} days ago`,
                });
            }
        }
    }
    const githubTabs = checkGitHub
        ? selectedTabs.filter((tab) => isGitHubIssueOrPr(tab.url) && isScriptableUrl(tab.url))
        : [];
    githubTotal = githubTabs.length;
    if (checkGitHub && githubTabs.length > 0) {
        let index = 0;
        const total = githubTabs.length;
        const workers = Array.from({ length: Math.min(githubConcurrency, total) }, async () => {
            while (true) {
                const currentIndex = index;
                if (currentIndex >= total) {
                    return;
                }
                index += 1;
                const tab = githubTabs[currentIndex];
                const state = await detectGitHubState(tab.tabId, githubTimeoutMs);
                githubChecked += 1;
                if (state === "closed" || state === "merged") {
                    githubMatched += 1;
                    addReason(tab, {
                        type: "closed_issue",
                        detail: `GitHub state: ${state}`,
                    });
                }
                if (progressEnabled) {
                    sendProgress(requestId, {
                        phase: "github",
                        processed: githubChecked,
                        total,
                        matched: githubMatched,
                        tabId: tab.tabId,
                        timeoutMs: githubTimeoutMs,
                    });
                }
            }
        });
        await Promise.all(workers);
    }
    const candidates = Array.from(candidateMap.values()).map((entry) => {
        const reasons = entry.reasons;
        const severity = reasons.some((reason) => reason.type === "duplicate" || reason.type === "closed_issue")
            ? "high"
            : "medium";
        return {
            tabId: entry.tab.tabId,
            windowId: entry.tab.windowId,
            groupId: entry.tab.groupId,
            url: entry.tab.url,
            title: entry.tab.title,
            lastFocusedAt: entry.tab.lastFocusedAt,
            reasons,
            severity,
        };
    });
    return {
        generatedAt: Date.now(),
        staleDays,
        totals: {
            tabs: scopeTabs.length,
            analyzed: selectedTabs.length,
            candidates: candidates.length,
        },
        meta: {
            durationMs: Date.now() - startedAt,
            githubChecked,
            githubTotal,
            githubMatched,
            githubTimeoutMs,
        },
        candidates,
    };
}
async function inspectTabs(params, requestId) {
    const signalList = Array.isArray(params.signals) && params.signals.length > 0
        ? params.signals.map(String)
        : ["page-meta"];
    const signalConcurrencyRaw = Number(params.signalConcurrency);
    const signalConcurrency = Number.isFinite(signalConcurrencyRaw) && signalConcurrencyRaw > 0
        ? Math.min(10, Math.floor(signalConcurrencyRaw))
        : 4;
    const signalTimeoutRaw = Number(params.signalTimeoutMs);
    const signalTimeoutMs = Number.isFinite(signalTimeoutRaw) && signalTimeoutRaw > 0
        ? Math.floor(signalTimeoutRaw)
        : 4000;
    const progressEnabled = params.progress === true;
    const snapshot = await getTabSnapshot();
    const selection = selectTabsByScope(snapshot, params);
    if (selection.error) {
        throw selection.error;
    }
    const tabs = selection.tabs;
    const startedAt = Date.now();
    const selectorSpecs = [];
    if (Array.isArray(params.selectorSpecs)) {
        selectorSpecs.push(...params.selectorSpecs);
    }
    if (params.signalConfig && typeof params.signalConfig === "object") {
        const config = params.signalConfig;
        if (Array.isArray(config.selectors)) {
            selectorSpecs.push(...config.selectors);
        }
        if (config.signals && typeof config.signals === "object") {
            const signals = config.signals;
            const selectorConfig = signals.selector;
            if (selectorConfig && Array.isArray(selectorConfig.selectors)) {
                selectorSpecs.push(...selectorConfig.selectors);
            }
        }
    }
    const normalizedSelectors = selectorSpecs
        .filter((spec) => spec && typeof spec.selector === "string" && spec.selector.length > 0)
        .map((spec) => ({
        name: typeof spec.name === "string" ? spec.name : undefined,
        selector: spec.selector,
        attr: typeof spec.attr === "string" ? spec.attr : "text",
        all: Boolean(spec.all),
    }));
    const signalDefs = [];
    for (const signalId of signalList) {
        if (signalId === "github-state") {
            signalDefs.push({
                id: signalId,
                match: (tab) => isGitHubIssueOrPr(tab.url) && isScriptableUrl(tab.url),
                run: async (tabId) => {
                    const state = await detectGitHubState(tabId, signalTimeoutMs);
                    return state ? { state } : null;
                },
            });
        }
        else if (signalId === "page-meta") {
            signalDefs.push({
                id: signalId,
                match: (tab) => isScriptableUrl(tab.url),
                run: async (tabId) => extractPageMeta(tabId, signalTimeoutMs),
            });
        }
        else if (signalId === "selector") {
            signalDefs.push({
                id: signalId,
                match: (tab) => isScriptableUrl(tab.url),
                run: async (tabId) => extractSelectorSignal(tabId, normalizedSelectors, signalTimeoutMs),
            });
        }
    }
    const tasks = [];
    for (const tab of tabs) {
        for (const signal of signalDefs) {
            if (signal.match(tab)) {
                tasks.push({ tab, signal });
            }
        }
    }
    const totalTasks = tasks.length;
    let completedTasks = 0;
    const entryMap = new Map();
    const workerCount = Math.min(signalConcurrency, totalTasks || 1);
    let index = 0;
    const workers = Array.from({ length: workerCount }, async () => {
        while (true) {
            const currentIndex = index;
            if (currentIndex >= totalTasks) {
                return;
            }
            index += 1;
            const task = tasks[currentIndex];
            const tabId = task.tab.tabId;
            let result = null;
            let error = null;
            const started = Date.now();
            try {
                result = await task.signal.run(tabId);
            }
            catch (err) {
                const message = err instanceof Error ? err.message : "signal_error";
                error = message;
            }
            const durationMs = Date.now() - started;
            const entry = entryMap.get(tabId) || { tab: task.tab, signals: {} };
            entry.signals[task.signal.id] = {
                ok: error === null,
                durationMs,
                data: result,
                error,
            };
            entryMap.set(tabId, entry);
            completedTasks += 1;
            if (progressEnabled) {
                sendProgress(requestId, {
                    phase: "inspect",
                    processed: completedTasks,
                    total: totalTasks,
                    signalId: task.signal.id,
                    tabId,
                });
            }
        }
    });
    await Promise.all(workers);
    const entries = Array.from(entryMap.values()).map((entry) => ({
        tabId: entry.tab.tabId,
        windowId: entry.tab.windowId,
        groupId: entry.tab.groupId,
        url: entry.tab.url,
        title: entry.tab.title,
        signals: entry.signals,
    }));
    return {
        generatedAt: Date.now(),
        totals: {
            tabs: tabs.length,
            signals: signalDefs.length,
            tasks: totalTasks,
        },
        meta: {
            durationMs: Date.now() - startedAt,
            signalTimeoutMs,
            selectorCount: normalizedSelectors.length,
        },
        entries,
    };
}
async function focusTab(params) {
    const tabIds = Array.isArray(params.tabIds) ? params.tabIds.map(Number) : [];
    const tabId = Number.isFinite(params.tabId)
        ? Number(params.tabId)
        : tabIds.length
            ? Number(tabIds[0])
            : null;
    if (!tabId) {
        throw new Error("Missing tabId");
    }
    const tab = await chrome.tabs.get(tabId);
    await chrome.windows.update(tab.windowId, { focused: true });
    await chrome.tabs.update(tabId, { active: true });
    return {
        tabId,
        windowId: tab.windowId,
    };
}
async function ensureArchiveWindow() {
    await ensureArchiveWindowIdLoaded();
    if (state.archiveWindowId) {
        try {
            await chrome.windows.get(state.archiveWindowId);
            return state.archiveWindowId;
        }
        catch {
            state.archiveWindowId = null;
        }
    }
    const created = await chrome.windows.create({ focused: false });
    state.archiveWindowId = created.id;
    await chrome.storage.local.set({ archiveWindowId: created.id });
    return created.id;
}
function buildWindowLabels(snapshot) {
    const labels = new Map();
    snapshot.windows.forEach((win, index) => {
        labels.set(win.windowId, `W${index + 1}`);
    });
    return labels;
}
function selectTabsByScope(snapshot, params) {
    const allTabs = flattenTabs(snapshot);
    if (params.tabIds && params.tabIds.length) {
        const idSet = new Set(params.tabIds.map(Number));
        return { tabs: allTabs.filter((tab) => idSet.has(tab.tabId)) };
    }
    if (params.groupId) {
        const groupId = Number(params.groupId);
        return { tabs: allTabs.filter((tab) => tab.groupId === groupId) };
    }
    if (params.groupTitle) {
        const matching = snapshot.windows.flatMap((win) => win.groups
            .filter((group) => group.title === params.groupTitle)
            .map((group) => ({ windowId: win.windowId, groupId: group.groupId })));
        if (matching.length === 0) {
            return { tabs: [], error: { message: "No matching group title found" } };
        }
        if (matching.length > 1 && !params.windowId) {
            return {
                tabs: [],
                error: {
                    message: "Group title is ambiguous. Provide a windowId.",
                    matches: matching,
                },
            };
        }
        const target = params.windowId
            ? matching.find((group) => group.windowId === Number(params.windowId))
            : matching[0];
        if (!target) {
            return { tabs: [], error: { message: "Group title not found in specified window" } };
        }
        return { tabs: allTabs.filter((tab) => tab.groupId === target.groupId) };
    }
    if (params.windowId) {
        const windowId = Number(params.windowId);
        return { tabs: allTabs.filter((tab) => tab.windowId === windowId) };
    }
    if (params.all) {
        return { tabs: allTabs };
    }
    const focusedWindow = snapshot.windows.find((win) => win.focused);
    if (!focusedWindow) {
        return { tabs: [] };
    }
    return { tabs: focusedWindow.tabs };
}
async function archiveTabs(params) {
    const snapshot = await getTabSnapshot();
    const windowLabels = buildWindowLabels(snapshot);
    let windowsToProcess = snapshot.windows;
    if (params.windowId) {
        windowsToProcess = windowsToProcess.filter((win) => win.windowId === Number(params.windowId));
    }
    else if (!params.all) {
        const focused = windowsToProcess.find((win) => win.focused);
        windowsToProcess = focused ? [focused] : [];
    }
    if (params.groupTitle || params.groupId || params.tabIds) {
        const selected = selectTabsByScope(snapshot, params);
        if (selected.error) {
            throw selected.error;
        }
        windowsToProcess = snapshot.windows
            .map((win) => ({
            windowId: win.windowId,
            focused: win.focused,
            state: win.state,
            tabs: win.tabs.filter((tab) => selected.tabs.some((sel) => sel.tabId === tab.tabId)),
            groups: win.groups,
        }))
            .filter((win) => win.tabs.length > 0);
    }
    if (windowsToProcess.length === 0) {
        return {
            txid: params.txid || null,
            summary: { movedTabs: 0, movedGroups: 0, skippedTabs: 0 },
            archiveWindowId: null,
            skipped: [],
            undo: { action: "archive", tabs: [] },
        };
    }
    const archiveWindowId = await ensureArchiveWindow();
    const undoTabs = [];
    const skipped = [];
    let movedGroups = 0;
    let movedTabs = 0;
    for (const window of windowsToProcess) {
        const groupsById = new Map();
        for (const group of window.groups) {
            groupsById.set(group.groupId, group);
        }
        const groupedTabs = new Map();
        const ungroupedTabs = [];
        for (const tab of window.tabs) {
            if (tab.tabId == null) {
                continue;
            }
            if (tab.groupId === -1) {
                ungroupedTabs.push(tab);
            }
            else {
                if (!groupedTabs.has(tab.groupId)) {
                    groupedTabs.set(tab.groupId, []);
                }
                groupedTabs.get(tab.groupId)?.push(tab);
            }
        }
        const windowLabel = windowLabels.get(window.windowId) || `W${window.windowId}`;
        const plans = [];
        for (const [groupId, tabs] of groupedTabs.entries()) {
            const group = groupsById.get(groupId) || null;
            plans.push({
                windowId: window.windowId,
                windowLabel,
                group,
                tabs,
                isUngrouped: false,
            });
        }
        if (ungroupedTabs.length > 0) {
            plans.push({
                windowId: window.windowId,
                windowLabel,
                group: null,
                tabs: ungroupedTabs,
                isUngrouped: true,
            });
        }
        for (const plan of plans) {
            const tabIds = plan.tabs.map((tab) => tab.tabId);
            if (tabIds.length === 0) {
                continue;
            }
            const tabById = new Map();
            for (const tab of plan.tabs) {
                tabById.set(tab.tabId, tab);
            }
            let moved;
            try {
                moved = await chrome.tabs.move(tabIds, { windowId: archiveWindowId, index: -1 });
            }
            catch (error) {
                for (const tabId of tabIds) {
                    skipped.push({ tabId, reason: "move_failed" });
                }
                log("Failed to move tabs", error);
                continue;
            }
            const movedList = Array.isArray(moved) ? moved : [moved];
            const movedIds = movedList.map((tab) => tab.id);
            movedTabs += movedIds.length;
            for (const movedId of movedIds) {
                const tab = tabById.get(movedId);
                if (!tab) {
                    continue;
                }
                undoTabs.push({
                    tabId: tab.tabId,
                    url: tab.url,
                    title: tab.title,
                    pinned: tab.pinned,
                    active: tab.active,
                    from: {
                        windowId: tab.windowId,
                        index: tab.index,
                        groupId: tab.groupId,
                        groupTitle: plan.group ? plan.group.title : null,
                        groupColor: plan.group ? plan.group.color : null,
                        groupCollapsed: plan.group ? plan.group.collapsed : null,
                    },
                });
            }
            const titleBase = plan.group && plan.group.title
                ? plan.group.title
                : plan.isUngrouped
                    ? "Ungrouped"
                    : "Group";
            const archiveTitle = `${plan.windowLabel} - ${titleBase}`;
            const groupColor = plan.group && plan.group.color
                ? plan.group.color
                : "grey";
            try {
                const newGroupId = await chrome.tabs.group({ tabIds: movedIds, createProperties: { windowId: archiveWindowId } });
                await chrome.tabGroups.update(newGroupId, { title: archiveTitle, color: groupColor });
                movedGroups += 1;
            }
            catch (error) {
                log("Failed to group archived tabs", error);
            }
        }
    }
    return {
        txid: params.txid || null,
        summary: {
            movedTabs,
            movedGroups,
            skippedTabs: skipped.length,
        },
        archiveWindowId,
        skipped,
        undo: {
            action: "archive",
            tabs: undoTabs,
        },
    };
}
async function getTabsByIds(tabIds) {
    const results = [];
    for (const tabId of tabIds) {
        try {
            const tab = await chrome.tabs.get(tabId);
            results.push(tab);
        }
        catch {
            results.push(null);
        }
    }
    return results;
}
async function closeTabs(params) {
    const mode = params.mode || "direct";
    if (mode === "direct" && !params.confirmed) {
        throw new Error("Direct close requires confirmation");
    }
    let tabIds = params.tabIds || [];
    if (!tabIds.length && (params.groupTitle || params.groupId || params.windowId)) {
        const snapshot = await getTabSnapshot();
        const selection = selectTabsByScope(snapshot, params);
        if (selection.error) {
            throw selection.error;
        }
        tabIds = selection.tabs.map((tab) => tab.tabId);
    }
    if (!tabIds.length) {
        return {
            txid: params.txid || null,
            summary: { closedTabs: 0, skippedTabs: 0 },
            skipped: [],
            undo: { action: "close", tabs: [] },
        };
    }
    const expectedUrls = params.expectedUrls || {};
    const tabInfos = await getTabsByIds(tabIds);
    const validTabs = [];
    const skipped = [];
    const groups = await chrome.tabGroups.query({});
    const groupById = new Map(groups.map((group) => [group.id, group]));
    for (let i = 0; i < tabIds.length; i += 1) {
        const tabId = tabIds[i];
        const tab = tabInfos[i];
        if (!tab) {
            skipped.push({ tabId, reason: "not_found" });
            continue;
        }
        const expected = expectedUrls[String(tabId)];
        if (expected && tab.url !== expected) {
            skipped.push({ tabId, reason: "url_mismatch" });
            continue;
        }
        const group = tab.groupId !== -1 ? groupById.get(tab.groupId) : null;
        validTabs.push({
            tabId,
            url: tab.url,
            title: tab.title,
            pinned: tab.pinned,
            active: tab.active,
            from: {
                windowId: tab.windowId,
                index: tab.index,
                groupId: tab.groupId,
                groupTitle: group ? group.title : null,
                groupColor: group ? group.color : null,
                groupCollapsed: group ? group.collapsed : null,
            },
        });
    }
    if (validTabs.length > 0) {
        await chrome.tabs.remove(validTabs.map((tab) => tab.tabId));
    }
    return {
        txid: params.txid || null,
        summary: {
            closedTabs: validTabs.length,
            skippedTabs: skipped.length,
        },
        skipped,
        undo: {
            action: "close",
            tabs: validTabs.map((tab) => ({
                url: tab.url,
                title: tab.title,
                pinned: tab.pinned,
                active: tab.active,
                from: tab.from,
            })),
        },
    };
}
async function extractDescription(tabId) {
    try {
        const [{ result }] = await chrome.scripting.executeScript({
            target: { tabId },
            func: () => {
                const pickContent = (selector) => {
                    const el = document.querySelector(selector);
                    if (!el) {
                        return "";
                    }
                    const content = el.getAttribute("content") || el.textContent || "";
                    return content.trim();
                };
                let description = pickContent("meta[name='description']") ||
                    pickContent("meta[property='og:description']") ||
                    pickContent("meta[name='twitter:description']");
                if (!description) {
                    const h1 = document.querySelector("h1");
                    const h1Text = h1 ? h1.textContent?.trim() : "";
                    const paragraphs = Array.from(document.querySelectorAll("p"))
                        .map((p) => p.textContent?.replace(/\s+/g, " ").trim() || "")
                        .filter((text) => text.length > 40);
                    const pText = paragraphs.length ? paragraphs[0] : "";
                    if (h1Text && pText) {
                        description = `${h1Text} - ${pText}`;
                    }
                    else {
                        description = h1Text || pText;
                    }
                }
                return description.replace(/\s+/g, " ").trim();
            },
        });
        if (!result) {
            return "";
        }
        return String(result).slice(0, DESCRIPTION_MAX_LENGTH);
    }
    catch {
        return "";
    }
}
async function reportTabs(params) {
    const snapshot = await getTabSnapshot();
    const selection = selectTabsByScope(snapshot, params);
    if (selection.error) {
        throw selection.error;
    }
    const windowLabels = buildWindowLabels(snapshot);
    const tabs = selection.tabs;
    const entries = [];
    for (const tab of tabs) {
        const description = isScriptableUrl(tab.url) ? await extractDescription(tab.tabId) : "";
        entries.push({
            tabId: tab.tabId,
            windowId: tab.windowId,
            windowLabel: windowLabels.get(tab.windowId) || `W${tab.windowId}`,
            groupId: tab.groupId,
            groupTitle: tab.groupTitle,
            groupColor: tab.groupColor,
            url: tab.url,
            title: tab.title,
            description,
            lastFocusedAt: tab.lastFocusedAt,
        });
    }
    return {
        generatedAt: Date.now(),
        entries,
        totals: {
            tabs: entries.length,
        },
    };
}
async function ensureWindow(windowId) {
    if (windowId) {
        try {
            const existing = await chrome.windows.get(windowId);
            if (existing) {
                return windowId;
            }
        }
        catch {
            // fall through to create
        }
    }
    const created = await chrome.windows.create({ focused: false });
    return created.id;
}
async function undoArchive(undo) {
    const tabs = undo.tabs || [];
    const restored = [];
    const skipped = [];
    const windowMap = new Map();
    for (const entry of tabs) {
        if (!entry.tabId) {
            skipped.push({ tabId: entry.tabId, reason: "missing_tab" });
            continue;
        }
        let targetWindowId = windowMap.get(entry.from.windowId);
        if (!targetWindowId) {
            targetWindowId = await ensureWindow(entry.from.windowId);
            windowMap.set(entry.from.windowId, targetWindowId);
        }
        try {
            await chrome.tabs.move(entry.tabId, { windowId: targetWindowId, index: -1 });
            restored.push({ tabId: entry.tabId, targetWindowId });
        }
        catch {
            skipped.push({ tabId: entry.tabId, reason: "move_failed" });
        }
    }
    const restoredSet = new Set(restored.map((item) => item.tabId));
    const groupsByWindow = new Map();
    for (const entry of tabs) {
        if (!restoredSet.has(entry.tabId)) {
            continue;
        }
        const targetWindowId = windowMap.get(entry.from.windowId) || entry.from.windowId;
        const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
        const hasGroupTitle = entry.from.groupTitle != null;
        if (!hasGroupId && !hasGroupTitle) {
            continue;
        }
        const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
        const key = `${targetWindowId}:${groupKey}`;
        if (!groupsByWindow.has(key)) {
            groupsByWindow.set(key, []);
        }
        groupsByWindow.get(key)?.push(entry);
    }
    for (const [key, groupTabs] of groupsByWindow.entries()) {
        const [windowIdPart] = key.split(":");
        const targetWindowId = Number(windowIdPart);
        const tabIds = groupTabs.map((entry) => entry.tabId).filter(Boolean);
        if (!tabIds.length) {
            continue;
        }
        try {
            const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
            await chrome.tabGroups.update(groupId, {
                title: groupTabs[0].from.groupTitle || "",
                color: groupTabs[0].from.groupColor || "grey",
                collapsed: groupTabs[0].from.groupCollapsed || false,
            });
        }
        catch (error) {
            log("Failed to recreate group", error);
        }
    }
    for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
        const windowTabs = tabs
            .filter((entry) => entry.from.windowId === originalWindowId && restoredSet.has(entry.tabId))
            .sort((a, b) => a.from.index - b.from.index);
        for (const entry of windowTabs) {
            if (!entry.tabId) {
                continue;
            }
            try {
                await chrome.tabs.move(entry.tabId, { windowId: targetWindowId, index: entry.from.index });
            }
            catch {
                // ignore ordering failures
            }
        }
        const activeTab = windowTabs.find((entry) => entry.active);
        if (activeTab && activeTab.tabId) {
            try {
                await chrome.tabs.update(activeTab.tabId, { active: true });
            }
            catch {
                // ignore
            }
        }
    }
    return {
        summary: {
            restoredTabs: restored.length,
            skippedTabs: skipped.length,
        },
        skipped,
    };
}
async function undoClose(undo) {
    const tabs = undo.tabs || [];
    const restored = [];
    const skipped = [];
    const windowMap = new Map();
    for (const entry of tabs) {
        if (!entry.url) {
            skipped.push({ url: entry.url, reason: "missing_url" });
            continue;
        }
        let targetWindowId = windowMap.get(entry.from.windowId);
        if (!targetWindowId) {
            targetWindowId = await ensureWindow(entry.from.windowId);
            windowMap.set(entry.from.windowId, targetWindowId);
        }
        try {
            const created = await chrome.tabs.create({
                windowId: targetWindowId,
                url: entry.url,
                active: false,
                pinned: entry.pinned,
            });
            restored.push({ tabId: created.id, entry });
        }
        catch {
            skipped.push({ url: entry.url, reason: "create_failed" });
        }
    }
    const groupsByWindow = new Map();
    for (const item of restored) {
        const entry = item.entry;
        const targetWindowId = windowMap.get(entry.from.windowId) || entry.from.windowId;
        const hasGroupId = entry.from.groupId != null && entry.from.groupId !== -1;
        const hasGroupTitle = entry.from.groupTitle != null;
        if (!hasGroupId && !hasGroupTitle) {
            continue;
        }
        const groupKey = hasGroupId ? entry.from.groupId : entry.from.groupTitle;
        const key = `${targetWindowId}:${groupKey}`;
        if (!groupsByWindow.has(key)) {
            groupsByWindow.set(key, []);
        }
        groupsByWindow.get(key)?.push({ tabId: item.tabId, entry });
    }
    for (const [key, groupTabs] of groupsByWindow.entries()) {
        const [windowIdPart] = key.split(":");
        const targetWindowId = Number(windowIdPart);
        const tabIds = groupTabs.map((item) => item.tabId).filter(Boolean);
        if (!tabIds.length) {
            continue;
        }
        try {
            const groupId = await chrome.tabs.group({ tabIds, createProperties: { windowId: targetWindowId } });
            await chrome.tabGroups.update(groupId, {
                title: groupTabs[0].entry.from.groupTitle || "",
                color: groupTabs[0].entry.from.groupColor || "grey",
                collapsed: groupTabs[0].entry.from.groupCollapsed || false,
            });
        }
        catch (error) {
            log("Failed to recreate group", error);
        }
    }
    for (const [originalWindowId, targetWindowId] of windowMap.entries()) {
        const windowTabs = restored
            .map((item) => ({ tabId: item.tabId, entry: item.entry }))
            .filter((item) => item.entry.from.windowId === originalWindowId)
            .sort((a, b) => a.entry.from.index - b.entry.from.index);
        for (const item of windowTabs) {
            try {
                await chrome.tabs.move(item.tabId, { windowId: targetWindowId, index: item.entry.from.index });
            }
            catch {
                // ignore ordering failures
            }
        }
        const activeTab = windowTabs.find((item) => item.entry.active);
        if (activeTab) {
            try {
                await chrome.tabs.update(activeTab.tabId, { active: true });
            }
            catch {
                // ignore
            }
        }
    }
    return {
        summary: {
            restoredTabs: restored.length,
            skippedTabs: skipped.length,
        },
        skipped,
    };
}
async function undoTransaction(params) {
    if (!params.record || !params.record.undo) {
        throw new Error("Undo record missing");
    }
    const undo = params.record.undo;
    if (undo.action === "archive") {
        return await undoArchive(undo);
    }
    if (undo.action === "close") {
        return await undoClose(undo);
    }
    throw new Error(`Unknown undo action: ${undo.action}`);
}
