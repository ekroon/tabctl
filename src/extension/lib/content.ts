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
