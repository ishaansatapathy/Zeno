/** One invalid field in a `422 validation_failed` answer. */
export type FieldIssue = { field: string; message: string };

/**
 * The API answered, and the answer was "no". `code` is stable and meant for logic and translation;
 * `message` is English prose for logs and fallbacks, never for concatenating into other strings.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly fields: FieldIssue[];
  /** The `x-request-id` of the exchange, to quote to support. */
  readonly requestId: string | undefined;
  /** Seconds to wait, from `Retry-After` on a `429`. */
  readonly retryAfterSeconds: number | undefined;

  constructor(init: {
    status: number;
    code: string;
    message: string;
    fields?: FieldIssue[];
    requestId?: string;
    retryAfterSeconds?: number;
  }) {
    super(init.message);
    this.name = "ApiError";
    this.status = init.status;
    this.code = init.code;
    this.fields = init.fields ?? [];
    this.requestId = init.requestId;
    this.retryAfterSeconds = init.retryAfterSeconds;
  }
}

/** The request never got an answer: the network, a timeout, or the caller giving up. */
export type NetworkFailure = "offline" | "timeout" | "aborted";

export class NetworkError extends Error {
  readonly reason: NetworkFailure;

  constructor(reason: NetworkFailure, message: string) {
    super(message);
    this.name = "NetworkError";
    this.reason = reason;
  }
}

export function isApiError(error: unknown, code?: string): error is ApiError {
  return error instanceof ApiError && (code === undefined || error.code === code);
}

export function isNetworkError(error: unknown): error is NetworkError {
  return error instanceof NetworkError;
}

/** The field-level messages of a validation error, keyed by field, for showing beside inputs. */
export function fieldMessages(error: unknown): Record<string, string> {
  const messages: Record<string, string> = {};
  if (error instanceof ApiError) {
    for (const issue of error.fields) {
      // The first message for a field is the most useful one.
      messages[issue.field] ??= issue.message;
    }
  }
  return messages;
}
