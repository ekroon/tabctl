/** JSON UTF-8 payload cap shared with the host's native-message framing boundary. */
export const MAX_NATIVE_MESSAGE_BYTES = 10 * 1024 * 1024;

type NativeMessage = Record<string, unknown>;
const encoder = new TextEncoder();

export function serializedBytes(value: unknown): number {
  return encoder.encode(JSON.stringify(value)).byteLength;
}

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Longest complete Unicode prefix whose JSON string contents fit the byte budget. */
function boundedJsonString(text: string, budget: number): string {
  let used = 0;
  let end = 0;
  while (end < text.length) {
    const unit = text.charCodeAt(end);
    let width = 1;
    let bytes: number;
    if (unit === 0x22 || unit === 0x5c || [8, 9, 10, 12, 13].includes(unit)) {
      bytes = 2;
    } else if (unit < 0x20) {
      bytes = 6;
    } else if (unit < 0x80) {
      bytes = 1;
    } else if (unit < 0x800) {
      bytes = 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff && end + 1 < text.length &&
        text.charCodeAt(end + 1) >= 0xdc00 && text.charCodeAt(end + 1) <= 0xdfff) {
      width = 2;
      bytes = 4;
    } else if (unit >= 0xd800 && unit <= 0xdfff) {
      bytes = 6; // JSON.stringify escapes unpaired UTF-16 surrogates.
    } else {
      bytes = 3;
    }
    if (used + bytes > budget) break;
    used += bytes;
    end += width;
  }
  return text.slice(0, end);
}

function budgetError(message: NativeMessage, originalBytes: number, maxBytes: number): NativeMessage {
  return {
    id: message.id,
    ...(message.action ? { action: message.action } : {}),
    ok: false,
    error: {
      message: `Native message exceeds the ${maxBytes}-byte transport budget (${originalBytes} bytes).`,
      hint: "Reduce scope or requested content size; no incomplete snapshot or screenshot was returned.",
    },
  };
}

/**
 * Bound only page HTML, retaining metadata and honest truncation diagnostics.
 * Other oversized results fail explicitly: clipping snapshots or base64 tiles corrupts their meaning.
 */
export function prepareNativeMessage(
  message: NativeMessage,
  maxBytes = MAX_NATIVE_MESSAGE_BYTES,
): NativeMessage {
  const originalBytes = serializedBytes(message);
  if (originalBytes <= maxBytes) return message;

  const data = object(message.data);
  const nested = object(data?.extraction);
  const extraction = nested || data;
  if (message.ok === true && extraction && typeof extraction.html === "string") {
    const html = extraction.html;
    const boundedExtraction = {
      ...extraction,
      html: "",
      truncatedHtml: true,
      transportTruncated: true,
      transportOriginalBytes: originalBytes,
      transportMaxBytes: maxBytes,
    };
    const bounded = {
      ...message,
      data: nested ? { ...data, extraction: boundedExtraction } : boundedExtraction,
    };
    const remaining = maxBytes - serializedBytes(bounded);
    if (remaining > 0) {
      boundedExtraction.html = boundedJsonString(html, remaining);
      if (boundedExtraction.html && serializedBytes(bounded) <= maxBytes) return bounded;
    }
  }

  const failure = budgetError(message, originalBytes, maxBytes);
  if (serializedBytes(failure) > maxBytes) {
    throw new Error("Native message identity exceeds the transport budget");
  }
  return failure;
}

export function postNativeMessage(port: chrome.runtime.Port, message: NativeMessage): NativeMessage {
  const bounded = prepareNativeMessage(message);
  port.postMessage(bounded);
  return bounded;
}
