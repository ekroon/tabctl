import { VERSION } from "../../shared/version";
import { getActiveProfile } from "../../shared/profiles";

export function printJson(payload: Record<string, unknown>, pretty = true): void {
  try {
    const active = getActiveProfile();
    if (active) {
      payload.profile = active.name;
      payload.browser = active.profile.browser;
    }
  } catch {
    // Don't let profile errors break output
  }
  const output = pretty ? JSON.stringify(payload, null, 2) : JSON.stringify(payload);
  process.stdout.write(`${output}\n`);
}

export function errorOut(message: string): never {
  const hints: Record<string, string> = {
    "Unknown option: --format": "Use --json for JSON output. --format is only for report.",
    "ENOENT": "Native host not running. Ensure the browser extension is loaded and active. If you recently upgraded, run: tabctl setup",
  };
  const hint = Object.entries(hints).find(([key]) => message.includes(key))?.[1];
  if (hint) {
    printJson({ ok: false, error: { message, hint } });
  } else {
    printJson({ ok: false, error: { message } });
  }
  process.exit(1);
  throw new Error(message);
}

export function setupStdoutErrorHandling(): void {
  process.stdout.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "EPIPE") {
      process.exit(0);
    }
    throw error;
  });
}

export function emitVersionWarnings(response: Record<string, unknown>, fallbackAction: string): void {
  const hostVersion = typeof response.version === "string" ? response.version : null;
  if (hostVersion && hostVersion !== VERSION) {
    process.stderr.write(`[tabctl] version mismatch: cli ${VERSION}, host ${hostVersion}. Run: tabctl setup\n`);
  }

  const data = response.data as Record<string, unknown> | undefined;
  const extensionVersion = data && typeof data.extensionVersion === "string" ? (data.extensionVersion as string) : null;
  const extensionComponent = data && typeof data.extensionComponent === "string" ? (data.extensionComponent as string) : null;
  if (extensionVersion && hostVersion && extensionVersion !== hostVersion) {
    process.stderr.write(`[tabctl] version mismatch: host ${hostVersion}, extension ${extensionVersion}. Reload the extension in your browser\n`);
  }
  if (extensionComponent && extensionComponent !== "extension") {
    process.stderr.write(`[tabctl] unexpected extension component: ${extensionComponent}\n`);
  }

  const action = (response.action as string | undefined) || fallbackAction;
  const extensionExpected = !["history", "version"].includes(action);
  if (extensionExpected && !extensionVersion) {
    process.stderr.write("[tabctl] extension version unavailable. Reload the extension in your browser\n");
  }
}
