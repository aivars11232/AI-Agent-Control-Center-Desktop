/**
 * Normalize any thrown/rejected value into readable text for the UI.
 *
 * Tauri command rejections do not arrive as `Error` instances — a backend
 * `Result::Err` is serialized to a plain object such as
 * `{ code, message, recoverable }` (persistence/policy/routing/review errors) or
 * a bare string. `String(value)` on those objects renders the useless
 * `"[object Object]"`, so unpack the known shapes first and only ever fall back
 * to a generic sentence.
 */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error.trim().length > 0 ? error : "An unexpected error occurred.";
  }
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    const message =
      typeof record.message === "string" && record.message.trim().length > 0
        ? record.message.trim()
        : null;
    const code =
      typeof record.code === "string" && record.code.trim().length > 0
        ? record.code.trim()
        : null;
    if (message && code) {
      return `${message} (${code})`;
    }
    if (message) {
      return message;
    }
    if (code) {
      return code;
    }
    try {
      const serialized = JSON.stringify(error);
      if (serialized && serialized !== "{}" && serialized !== "null") {
        return serialized;
      }
    } catch {
      // fall through to the generic fallback below
    }
  }
  return "An unexpected error occurred.";
}
