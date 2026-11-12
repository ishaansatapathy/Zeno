import { ApiError, NetworkError } from "./errors";

/** Which message in the `errors` namespace explains a failure, and what to fill into it. */
export type ErrorMessage = { key: string; values?: Record<string, string | number> };

/** Codes the app has a written explanation for (see `messages/*.json`, namespace `errors`). */
const explained = new Set([
  "invalid_credentials",
  "email_not_verified",
  "validation_failed",
  "unauthenticated",
  "session_expired",
  "payload_too_large",
  "internal_error",
]);

/**
 * What to tell the person when a call fails. Never the server's English prose: the `code` picks a
 * message in `messages/`, so the wording lives in one place and a second language needs no code.
 */
export function errorMessage(error: unknown): ErrorMessage {
  if (error instanceof NetworkError) return { key: error.reason };
  if (error instanceof ApiError) {
    if (error.code === "too_many_requests") {
      return error.retryAfterSeconds === undefined
        ? { key: "too_many_requests_soon" }
        : { key: "too_many_requests", values: { seconds: error.retryAfterSeconds } };
    }
    if (explained.has(error.code)) return { key: error.code };
    if (error.status >= 500) return { key: "internal_error" };
  }
  return { key: "unknown" };
}
