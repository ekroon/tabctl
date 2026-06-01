// Content script execution utilities — extracted from background.ts (pure structural refactor).

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

export type ScriptExecutionResult<T> =
  | { kind: "ok"; value: T | null }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

export async function executeWithTimeoutDetailed<T>(
  tabId: number,
  timeoutMs: number,
  func: (...args: Array<any>) => T | Promise<T>,
  args: Array<unknown> = [],
): Promise<ScriptExecutionResult<T>> {
  const execPromise: Promise<ScriptExecutionResult<T>> = chrome.scripting.executeScript({
    target: { tabId },
    func,
    args,
  }).then((result): ScriptExecutionResult<T> => {
    if (!result || !Array.isArray(result)) {
      return { kind: "ok", value: null };
    }
    const [{ result: value }] = result as Array<{ result?: T | null }>;
    return { kind: "ok", value: value ?? null };
  }).catch((err: unknown) => ({
    kind: "error",
    message: err instanceof Error ? err.message : String(err),
  }));

  const timeoutPromise = new Promise<ScriptExecutionResult<T>>((resolve) => {
    const handle = setTimeout(() => {
      clearTimeout(handle);
      resolve({ kind: "timeout" });
    }, timeoutMs);
  });

  return Promise.race([execPromise, timeoutPromise]);
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

export async function extractPageMarkdown(tabId: number, timeoutMs: number, maxHtmlChars: number) {
  const result = await executeWithTimeout(tabId, timeoutMs, (cap: number) => {
    const raw = document.documentElement?.outerHTML || "";
    return raw.length > cap ? raw.slice(0, cap) : raw;
  }, [maxHtmlChars]);

  return typeof result === "string" ? result : "";
}

type PageHtmlPayload = {
  html: string;
  sourceHtmlChars: number;
  sourceTextChars: number;
  documentReadyState: string;
  truncatedHtml: boolean;
};

type PageQuiescenceCounters = {
  textChars: number;
  htmlChars: number;
  domElements: number;
  resourceCount: number | null;
};

export type PageQuiescenceProbe = {
  quiet: boolean;
  reason: string;
  documentReadyState: string | null;
  before: PageQuiescenceCounters | null;
  after: PageQuiescenceCounters | null;
  elapsedMs: number;
  error: string | null;
};

function injectionStatus(message: string) {
  const lower = message.toLowerCase();
  if (lower.includes("cannot access") || lower.includes("permission") || lower.includes("extensions gallery")) {
    return "PROTECTED";
  }
  return "INJECTION_FAILED";
}

export async function extractPageHtml(tabId: number, timeoutMs: number, maxHtmlChars: number) {
  const result = await executeWithTimeoutDetailed<PageHtmlPayload>(tabId, timeoutMs, (cap: number) => {
    const raw = document.documentElement?.outerHTML || "";
    const text = document.body?.innerText || document.documentElement?.textContent || "";
    return {
      html: raw.length > cap ? raw.slice(0, cap) : raw,
      sourceHtmlChars: raw.length,
      sourceTextChars: text.length,
      documentReadyState: document.readyState,
      truncatedHtml: raw.length > cap,
    };
  }, [maxHtmlChars]);

  if (result.kind === "timeout") {
    return {
      status: "TIMED_OUT",
      html: "",
      sourceHtmlChars: 0,
      sourceTextChars: 0,
      documentReadyState: null,
      truncatedHtml: false,
      error: `Timed out after ${timeoutMs}ms`,
    };
  }

  if (result.kind === "error") {
    return {
      status: injectionStatus(result.message),
      html: "",
      sourceHtmlChars: 0,
      sourceTextChars: 0,
      documentReadyState: null,
      truncatedHtml: false,
      error: result.message,
    };
  }

  if (!result.value) {
    return {
      status: "EXTRACTION_FAILED",
      html: "",
      sourceHtmlChars: 0,
      sourceTextChars: 0,
      documentReadyState: null,
      truncatedHtml: false,
      error: "Content script returned no page HTML payload",
    };
  }

  const status = result.value.documentReadyState === "loading" ? "NOT_LOADED" : "READ";
  return {
    status,
    ...result.value,
    error: null,
  };
}

export async function probePageQuiescence(tabId: number, timeoutMs: number, sampleWindowMs: number) {
  const result = await executeWithTimeoutDetailed<PageQuiescenceProbe>(tabId, timeoutMs, async (rawWindowMs: number) => {
    const startedAt = Date.now();
    const windowMs = Math.max(50, Math.min(Number(rawWindowMs) || 350, 1_500));
    const htmlCap = 250_000;

    const sample = (): PageQuiescenceCounters => {
      const root = document.documentElement;
      const bodyText = document.body?.innerText || root?.textContent || "";
      const html = root?.outerHTML || "";
      const resources = typeof performance !== "undefined" && typeof performance.getEntriesByType === "function"
        ? performance.getEntriesByType("resource").length
        : null;
      return {
        textChars: bodyText.length,
        htmlChars: Math.min(html.length, htmlCap),
        domElements: document.getElementsByTagName("*").length,
        resourceCount: resources,
      };
    };

    const waitForIdleWindow = () => new Promise<void>((resolve) => {
      const win = window as Window & {
        requestIdleCallback?: (callback: () => void, options?: { timeout: number }) => number;
      };
      const done = () => setTimeout(resolve, windowMs);
      if (typeof win.requestIdleCallback === "function") {
        let resolved = false;
        const finish = () => {
          if (resolved) {
            return;
          }
          resolved = true;
          done();
        };
        const fallback = setTimeout(finish, windowMs + 100);
        win.requestIdleCallback(() => {
          clearTimeout(fallback);
          finish();
        }, { timeout: windowMs });
        return;
      }
      setTimeout(resolve, windowMs);
    });

    try {
      const readyState = document.readyState;
      if (readyState !== "interactive" && readyState !== "complete") {
        return {
          quiet: false,
          reason: "not-ready",
          documentReadyState: readyState,
          before: null,
          after: null,
          elapsedMs: Date.now() - startedAt,
          error: null,
        };
      }

      const before = sample();
      await waitForIdleWindow();
      const after = sample();
      const stable = before.textChars === after.textChars
        && before.htmlChars === after.htmlChars
        && before.domElements === after.domElements
        && before.resourceCount === after.resourceCount;

      return {
        quiet: stable,
        reason: stable ? "stable" : "changed",
        documentReadyState: document.readyState,
        before,
        after,
        elapsedMs: Date.now() - startedAt,
        error: null,
      };
    } catch (err) {
      return {
        quiet: false,
        reason: "probe-error",
        documentReadyState: typeof document !== "undefined" ? document.readyState : null,
        before: null,
        after: null,
        elapsedMs: Date.now() - startedAt,
        error: err instanceof Error ? err.message : String(err),
      };
    }
  }, [sampleWindowMs]);

  if (result.kind === "timeout") {
    return {
      quiet: false,
      reason: "timed-out",
      documentReadyState: null,
      before: null,
      after: null,
      elapsedMs: timeoutMs,
      error: `Timed out after ${timeoutMs}ms`,
    };
  }
  if (result.kind === "error") {
    return {
      quiet: false,
      reason: "injection-error",
      documentReadyState: null,
      before: null,
      after: null,
      elapsedMs: 0,
      error: result.message,
    };
  }
  return result.value ?? {
    quiet: false,
    reason: "no-result",
    documentReadyState: null,
    before: null,
    after: null,
    elapsedMs: 0,
    error: "Content script returned no quiescence probe payload",
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
    const stringCap = typeof maxLen === "number" && maxLen > 0 ? maxLen : 500;
    const htmlCap = typeof maxLen === "number" && maxLen > 0 ? maxLen : 4096;
    const normalizeStringValue = (value: string, cap: number) => value.replace(/\s+/g, " ").trim().slice(0, cap);

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
      const styleProps = Array.isArray(raw.styleProps)
        ? raw.styleProps.filter((prop): prop is string => typeof prop === "string").map((prop) => prop.trim()).filter(Boolean)
        : [];
      if (!textModes.has(normalizedTextMode)) {
        errors[name] = `Unsupported textMode: ${textMode || "unknown"}`;
        hints[name] = "Use textMode: contains | exact | starts-with";
        continue;
      }

      try {
        const elements = Array.from(document.querySelectorAll(selector));
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
        if (attr === "count") {
          values[name] = filtered.length;
          continue;
        }

        if (!elements.length) {
          missing.push(name);
          if (selector.includes(":contains(")) {
            hints[name] = "CSS :contains() is not supported; use selector text filters or a different selector.";
          } else {
            hints[name] = "No matches found; capture a screenshot for context or adjust the selector.";
          }
          continue;
        }

        if (!filtered.length) {
          missing.push(name);
          hints[name] = "Selector matched elements, but none matched the text filter; capture a screenshot for context or adjust text/textMode.";
          continue;
        }

        const getValue = (el: Element): string | number | boolean | Record<string, unknown> | null => {
          if (attr === "text") {
            return normalizeStringValue(el.textContent || "", stringCap);
          }
          if (attr === "html") {
            return normalizeStringValue(el.outerHTML || "", htmlCap);
          }
          if (attr === "href-url" || attr === "src-url") {
            const rawValue = el.getAttribute(attr === "href-url" ? "href" : "src") || "";
            if (!rawValue) {
              return "";
            }
            try {
              const resolved = new URL(rawValue, document.baseURI);
              if (resolved.protocol === "http:" || resolved.protocol === "https:") {
                return normalizeStringValue(resolved.toString(), stringCap);
              }
              return "";
            } catch {
              return "";
            }
          }
          if (attr === "value") {
            if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
              return el.value;
            }
            return null;
          }
          if (attr === "box") {
            const rect = el.getBoundingClientRect();
            return {
              x: rect.x,
              y: rect.y,
              width: rect.width,
              height: rect.height,
              top: rect.top,
              right: rect.right,
              bottom: rect.bottom,
              left: rect.left,
            };
          }
          if (attr === "styles") {
            if (!styleProps.length) {
              return {};
            }
            const computed = window.getComputedStyle(el);
            const selected: Record<string, unknown> = {};
            for (const prop of styleProps) {
              selected[prop] = computed.getPropertyValue(prop);
            }
            return selected;
          }
          if (attr === "visible") {
            const computed = window.getComputedStyle(el);
            const rect = el.getBoundingClientRect();
            const styleVisible = computed.display !== "none" && computed.visibility !== "hidden" && computed.opacity !== "0";
            const hasRenderedBox = rect.width > 0 || rect.height > 0;
            return styleVisible && hasRenderedBox;
          }
          if (attr === "enabled") {
            const candidate = el as Element & { disabled?: boolean };
            return candidate.disabled !== true && el.getAttribute("aria-disabled") !== "true";
          }
          if (attr === "checked") {
            if (el instanceof HTMLInputElement) {
              return el.checked;
            }
            return el.getAttribute("aria-checked") === "true";
          }
          return normalizeStringValue(el.getAttribute(attr) || "", stringCap);
        };

        if (attr === "box") {
          values[name] = getValue(filtered[0]);
          continue;
        }

        if (all) {
          values[name] = filtered.map(getValue).filter((val) => typeof val !== "string" || val.length > 0);
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
