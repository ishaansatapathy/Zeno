import { ApiError, NetworkError, type FieldIssue } from "./errors";

/**
 * A small typed `fetch` wrapper for the Zeno API.
 *
 * - The session lives in HttpOnly cookies the browser sends by itself (`credentials: "include"`);
 *   nothing is ever read from or written to `localStorage`.
 * - The API's CSRF rule is the `Origin` header, which browsers add to every cross-origin write.
 *   Nothing to do here except never to strip it.
 * - A `401` on a normal call means the access cookie expired: one refresh is made (however many
 *   calls failed at once), then each failed call is repeated once.
 * - Errors come back as `ApiError` (the API answered) or `NetworkError` (it did not).
 */

export type ClientOptions = {
  /** Where the API is, without a trailing slash. `""` for same-origin. */
  baseUrl: string;
  fetch?: typeof fetch;
  /** Give up on one request after this long. Default 15 s. */
  timeoutMs?: number;
  /** Called once when the session cannot be renewed: the user has to sign in again. */
  onSessionExpired?: () => void;
  /** For tests. */
  requestId?: () => string;
};

export type RequestOptions = {
  query?: Record<string, string | number | boolean | undefined | null>;
  body?: unknown;
  signal?: AbortSignal;
  /** Makes a repeated POST (a checkout, say) safe: the API returns the first answer again. */
  idempotencyKey?: string;
};

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

const REFRESH_PATH = "/v1/auth/refresh";
/** Auth calls where a `401` means "wrong credentials" or "no session", not "expired". */
const NEVER_REFRESH = [
  "/v1/auth/login",
  "/v1/auth/register",
  "/v1/auth/refresh",
  "/v1/auth/logout",
  "/v1/auth/verify-email",
  "/v1/auth/resend-verification",
  "/v1/auth/forgot-password",
  "/v1/auth/reset-password",
];

export function newRequestId(): string {
  return globalThis.crypto.randomUUID();
}

function queryString(query: RequestOptions["query"]): string {
  if (!query) return "";
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== null) params.set(key, String(value));
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

function parseRetryAfter(header: string | null): number | undefined {
  if (header === null) return undefined;
  const seconds = Number(header);
  return Number.isFinite(seconds) && seconds >= 0 ? Math.ceil(seconds) : undefined;
}

async function readError(response: Response, requestId: string | undefined): Promise<ApiError> {
  let code = `http_${response.status}`;
  let message = "The request failed.";
  let fields: FieldIssue[] = [];
  try {
    const body: unknown = await response.json();
    const detail = (body as { error?: Record<string, unknown> } | null)?.error;
    if (detail && typeof detail === "object") {
      if (typeof detail.code === "string") code = detail.code;
      if (typeof detail.message === "string") message = detail.message;
      if (Array.isArray(detail.fields)) {
        fields = detail.fields.filter(
          (f): f is FieldIssue =>
            typeof f === "object" &&
            f !== null &&
            typeof (f as FieldIssue).field === "string" &&
            typeof (f as FieldIssue).message === "string",
        );
      }
    }
  } catch {
    // A proxy's HTML error page, an empty body: the status is all there is.
  }
  return new ApiError({
    status: response.status,
    code,
    message,
    fields,
    requestId,
    retryAfterSeconds: parseRetryAfter(response.headers.get("retry-after")),
  });
}

export type Client = ReturnType<typeof createClient>;

export function createClient(options: ClientOptions) {
  const doFetch = options.fetch ?? ((...args: Parameters<typeof fetch>) => fetch(...args));
  const timeoutMs = options.timeoutMs ?? 15_000;
  const makeRequestId = options.requestId ?? newRequestId;
  /** The refresh in progress, shared by everything that fails while it runs. */
  let refreshing: Promise<boolean> | null = null;

  /** One round trip, no refresh logic. */
  async function send(
    method: Method,
    path: string,
    opts: RequestOptions,
    payload: string | undefined,
  ): Promise<Response & { requestId: string }> {
    const requestId = makeRequestId();
    const headers: Record<string, string> = {
      accept: "application/json",
      "x-request-id": requestId,
    };
    if (payload !== undefined) headers["content-type"] = "application/json";
    if (opts.idempotencyKey) headers["idempotency-key"] = opts.idempotencyKey;

    // Nothing to send if the caller has already given up.
    if (opts.signal?.aborted) throw new NetworkError("aborted", "The request was cancelled.");
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
    const onCallerAbort = () => controller.abort();
    opts.signal?.addEventListener("abort", onCallerAbort, { once: true });
    try {
      const response = await doFetch(`${options.baseUrl}${path}${queryString(opts.query)}`, {
        method,
        headers,
        body: payload,
        credentials: "include",
        signal: controller.signal,
      });
      return Object.assign(response, { requestId });
    } catch (error) {
      if (timedOut) throw new NetworkError("timeout", "The request took too long.");
      if (controller.signal.aborted) throw new NetworkError("aborted", "The request was cancelled.");
      throw new NetworkError(
        "offline",
        error instanceof Error ? error.message : "The network request failed.",
      );
    } finally {
      clearTimeout(timer);
      opts.signal?.removeEventListener("abort", onCallerAbort);
    }
  }

  /** Renews the session. Resolves `true` if it was renewed, `false` if it could not be. */
  function refresh(): Promise<boolean> {
    refreshing ??= (async () => {
      try {
        const response = await send("POST", REFRESH_PATH, {}, undefined);
        // Said here, not by each failed call, so ten calls that fail together are one sign-out.
        if (!response.ok) options.onSessionExpired?.();
        return response.ok;
      } finally {
        // Whatever happened, the next expiry makes a fresh attempt.
        refreshing = null;
      }
    })();
    return refreshing;
  }

  async function request<T>(method: Method, path: string, opts: RequestOptions = {}): Promise<T> {
    // Serialised once, so a repeat after a refresh sends exactly what the first try did.
    const payload = opts.body === undefined ? undefined : JSON.stringify(opts.body);
    let response = await send(method, path, opts, payload);

    if (response.status === 401 && !NEVER_REFRESH.includes(path)) {
      // A network failure while refreshing is not "signed out": it propagates as itself.
      const renewed = await refresh();
      if (!renewed) throw await readError(response, response.requestId);
      // The first answer is not needed any more; let the connection go.
      await response.body?.cancel();
      response = await send(method, path, opts, payload);
    }

    if (!response.ok) throw await readError(response, response.requestId);
    if (response.status === 204) return undefined as T;
    try {
      return (await response.json()) as T;
    } catch {
      throw new ApiError({
        status: response.status,
        code: "invalid_response",
        message: "The server's answer could not be read.",
        requestId: response.requestId,
      });
    }
  }

  return {
    get: <T>(path: string, opts?: Omit<RequestOptions, "body">) => request<T>("GET", path, opts),
    post: <T>(path: string, opts?: RequestOptions) => request<T>("POST", path, opts),
    put: <T>(path: string, opts?: RequestOptions) => request<T>("PUT", path, opts),
    patch: <T>(path: string, opts?: RequestOptions) => request<T>("PATCH", path, opts),
    delete: <T>(path: string, opts?: RequestOptions) => request<T>("DELETE", path, opts),
  };
}
