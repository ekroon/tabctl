// Content script execution utilities — extracted from background.ts (pure structural refactor).

export const SETTLE_STABILITY_MS = 500;
export const SETTLE_POLL_INTERVAL_MS = 50;

export function isScriptableUrl(url: unknown) {
  return typeof url === "string" && (url.startsWith("http://") || url.startsWith("https://"));
}

export function delay(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function executeWithTimeout<T>(
  tabId: number,
  timeoutMs: number,
  func: (...args: Array<any>) => T,
  args: Array<unknown> = [],
): Promise<T | null> {
  const execPromise = chrome.scripting.executeScript({
    target: { tabId },
    func,
    args,
  });

  const timeoutPromise = new Promise<null>((resolve) => {
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
    const [{ result: value }] = result as Array<{ result?: T | null }>;
    return value ?? null;
  } catch {
    return null;
  }
}

export function isGitHubIssueOrPr(url: string | null) {
  if (!url) {
    return false;
  }
  return /^https:\/\/github\.com\/[^/]+\/[^/]+\/(issues|pull)\/\d+/.test(url);
}

export async function detectGitHubState(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    const stateEl =
      document.querySelector(".gh-header-meta .State") ||
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

export async function extractPageMeta(tabId: number, timeoutMs: number, descriptionMaxLength: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    const pickContent = (selector: string) => {
      const el = document.querySelector(selector);
      if (!el) {
        return "";
      }
      const content = el.getAttribute("content") || el.textContent || "";
      return content.trim();
    };

    const description =
      pickContent("meta[name='description']") ||
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

  const meta = result as { description?: string; h1?: string };
  return {
    description: (meta.description || "").slice(0, descriptionMaxLength),
    h1: (meta.h1 || "").slice(0, descriptionMaxLength),
  };
}

export async function extractSelectorSignal(tabId: number, specs: Array<Record<string, unknown>>, timeoutMs: number, selectorValueMaxLength: number) {
  if (!specs.length) {
    return null;
  }

  const result = await executeWithTimeout(tabId, timeoutMs, (rawSpecs: Array<Record<string, unknown>>, maxLen: number) => {
    const values: Record<string, unknown> = {};
    const missing: string[] = [];
    const errors: Record<string, string> = {};
    const hints: Record<string, string> = {};

    for (const raw of rawSpecs) {
      const selector = typeof raw.selector === "string" ? raw.selector : "";
      if (!selector) {
        continue;
      }
      const name = typeof raw.name === "string" && raw.name ? raw.name : selector;
      const attr = typeof raw.attr === "string" ? raw.attr : "text";
      const all = Boolean(raw.all);
      const text = typeof raw.text === "string" ? raw.text.trim() : "";
      const textMode = typeof raw.textMode === "string" ? raw.textMode.trim().toLowerCase() : "";
      const normalizedTextMode = textMode === "includes" ? "contains" : textMode;
      const textModes = new Set(["", "contains", "exact", "starts-with"]);
      if (!textModes.has(normalizedTextMode)) {
        errors[name] = `Unsupported textMode: ${textMode || "unknown"}`;
        hints[name] = "Use textMode: contains | exact | starts-with";
        continue;
      }

      try {
        const elements = Array.from(document.querySelectorAll(selector));
        if (!elements.length) {
          missing.push(name);
          if (selector.includes(":contains(")) {
            hints[name] = "CSS :contains() is not supported; use selector text filters or a different selector.";
          } else {
            hints[name] = "No matches found; capture a screenshot for context or adjust the selector.";
          }
          continue;
        }

        const matchesText = (el: Element) => {
          if (!text) {
            return true;
          }
          const content = (el.textContent || "").replace(/\s+/g, " ").trim();
          if (normalizedTextMode === "exact") {
            return content === text;
          }
          if (normalizedTextMode === "starts-with") {
            return content.startsWith(text);
          }
          return content.includes(text);
        };

        const filtered = text ? elements.filter(matchesText) : elements;
        if (!filtered.length) {
          missing.push(name);
          hints[name] = "Selector matched elements, but none matched the text filter; capture a screenshot for context or adjust text/textMode.";
          continue;
        }

        const getValue = (el: Element) => {
          let value = "";
          if (attr === "text") {
            value = el.textContent || "";
          } else if (attr === "href-url" || attr === "src-url") {
            const rawValue = el.getAttribute(attr === "href-url" ? "href" : "src") || "";
            if (!rawValue) {
              value = "";
            } else {
              try {
                const resolved = new URL(rawValue, document.baseURI);
                if (resolved.protocol === "http:" || resolved.protocol === "https:") {
                  value = resolved.toString();
                } else {
                  value = "";
                }
              } catch {
                value = "";
              }
            }
          } else {
            value = el.getAttribute(attr) || "";
          }
          return value.replace(/\s+/g, " ").trim().slice(0, maxLen);
        };

        if (all) {
          values[name] = filtered.map(getValue).filter((val) => val.length > 0);
        } else {
          values[name] = getValue(filtered[0]);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "selector_error";
        errors[name] = message;
        if (selector.includes(":contains(")) {
          hints[name] = "CSS :contains() is not supported; use selector text filters or a different selector.";
        } else {
          hints[name] = "Selector failed to evaluate; capture a screenshot for context or adjust the selector.";
        }
      }
    }

    return { values, missing, errors, hints };
  }, [specs, selectorValueMaxLength]);

  if (!result || typeof result !== "object") {
    return null;
  }

  return result as Record<string, unknown>;
}

export function waitForTabLoad(tabId: number, timeoutMs: number) {
  return new Promise<void>((resolve) => {
    let settled = false;
    const done = () => {
      if (settled) {
        return;
      }
      settled = true;
      chrome.tabs.onUpdated.removeListener(onUpdated);
      resolve();
    };

    const onUpdated = (updatedTabId: number, info: chrome.tabs.TabChangeInfo) => {
      if (updatedTabId === tabId && info.status === "complete") {
        done();
      }
    };

    chrome.tabs.onUpdated.addListener(onUpdated);
    chrome.tabs.get(tabId).then((tab) => {
      if (tab.status === "complete") {
        done();
      }
    }).catch(() => {
      done();
    });

    setTimeout(done, timeoutMs);
  });
}

export async function waitForDomReady(tabId: number, timeoutMs: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, () => {
    if (document.readyState === "interactive" || document.readyState === "complete") {
      return true;
    }
    return new Promise<boolean>((resolve) => {
      const onReady = () => {
        document.removeEventListener("DOMContentLoaded", onReady);
        resolve(true);
      };
      document.addEventListener("DOMContentLoaded", onReady, { once: true });
      setTimeout(() => {
        document.removeEventListener("DOMContentLoaded", onReady);
        resolve(false);
      }, Math.max(0, timeoutMs - 50));
    });
  });

  if (result === null) {
    await delay(Math.min(200, Math.max(50, Math.floor(timeoutMs / 10))));
  }
}

export async function waitForSettle(tabId: number, timeoutMs: number): Promise<void> {
  const startTime = Date.now();
  let lastUrl = "";
  let lastTitle = "";
  let stableStart = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    const tab = await chrome.tabs.get(tabId).catch(() => null);
    if (!tab) return;

    const currentUrl = tab.url || "";
    const currentTitle = tab.title || "";

    // Reset stability timer if URL or title changed
    if (currentUrl !== lastUrl || currentTitle !== lastTitle) {
      lastUrl = currentUrl;
      lastTitle = currentTitle;
      stableStart = Date.now();
    } else if (
      isScriptableUrl(currentUrl) &&
      tab.status === "complete" &&
      Date.now() - stableStart >= SETTLE_STABILITY_MS
    ) {
      // Page is loaded, URL is valid, and stable for long enough
      return;
    }

    await delay(SETTLE_POLL_INTERVAL_MS);
  }
  // Timeout reached, continue anyway
}

export async function waitForTabReady(tabId: number, params: Record<string, unknown>, fallbackTimeoutMs: number) {
  const waitFor = typeof params.waitFor === "string" ? params.waitFor.trim().toLowerCase() : "";
  if (!waitFor || waitFor === "none") {
    return;
  }
  const timeoutRaw = Number(params.waitTimeoutMs);
  const timeoutMs = Number.isFinite(timeoutRaw) && timeoutRaw > 0 ? Math.floor(timeoutRaw) : fallbackTimeoutMs;

  // settle mode handles its own URL checking, so skip the early return
  if (waitFor === "settle") {
    await waitForSettle(tabId, timeoutMs);
    return;
  }

  try {
    const tab = await chrome.tabs.get(tabId);
    if (!isScriptableUrl(tab.url)) {
      return;
    }
  } catch {
    return;
  }

  if (waitFor === "load") {
    await waitForTabLoad(tabId, timeoutMs);
    return;
  }

  if (waitFor === "dom") {
    await waitForDomReady(tabId, timeoutMs);
  }
}
