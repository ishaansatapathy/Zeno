// The one way the web app talks to the API.
//
// Types come from the OpenAPI document (`schema.d.ts`), so a route, a field or a status the API no
// longer has is a type error here. Calls go to the page's own origin under `/v1`: in development
// Next proxies that path to the API (see next.config.mjs), in production the edge does. Same
// origin means the session cookies belong to the page, need no CORS, and the `Origin` the API
// checks is the page's own.
//
// Two things happen below the call sites, so no screen has to remember them:
//
// * A session lasts minutes. When a request answers `401 unauthenticated`, the session is renewed
//   once and the request is repeated. Many requests failing at once renew it once, and so do many
//   tabs (a Web Lock), because a refresh token works only once.
// * Every failure comes back in one shape (`ApiFailure`), including a server that cannot be
//   reached, so a screen never has to parse an error body or catch a network exception.

import createClient from "openapi-fetch";

import type { components, paths } from "./schema";

export type Schemas = components["schemas"];

/** What went wrong, in one shape. `status` is 0 when the server could not be reached at all. */
export interface ApiFailure {
  status: number;
  /** Stable and machine-readable: branch on this, not on the message. */
  code: string;
  /** Written for people; safe to show. */
  message: string;
  /** For `validation_failed`: the message for each invalid field. */
  fields: Record<string, string>;
  /** For `too_many_requests`: seconds until trying again can work. */
  retryAfter: number | null;
}

export type Result<T> = { ok: true; data: T } | { ok: false; error: ApiFailure };

/** Routes where a 401 is the answer itself, not a session that needs renewing. */
const NO_RENEWAL = /^\/v1\/auth\/(login|register|verify-email|resend-verification|forgot-password|reset-password|refresh|logout)$/;

const NETWORK: ApiFailure = {
  status: 0,
  code: "network",
  message: "We couldn't reach Zeno. Check your connection and try again.",
  fields: {},
  retryAfter: null,
};

export interface ClientOptions {
  /** Defaults to the page's own origin. */
  baseUrl?: string;
  /** Defaults to `globalThis.fetch`. */
  fetch?: typeof fetch;
  /** Runs `task` while no other tab renews the session. Defaults to the Web Locks API. */
  withLock?: <T>(task: () => Promise<T>) => Promise<T>;
  /** How long to wait before repeating a request that lost a refresh race. */
  raceDelayMs?: number;
}

function webLock<T>(task: () => Promise<T>): Promise<T> {
  const locks = typeof navigator === "undefined" ? undefined : navigator.locks;
  return locks ? locks.request("zeno-session-refresh", task) : task();
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function errorCode(response: Response): Promise<string> {
  try {
    const body = (await response.clone().json()) as { error?: { code?: string } };
    return body.error?.code ?? "";
  } catch {
    return "";
  }
}

/** Turns whatever the server sent into an `ApiFailure`. */
export async function failureOf(response: Response, body?: unknown): Promise<ApiFailure> {
  const error = (body as { error?: { code?: unknown; message?: unknown; fields?: unknown } } | undefined)?.error;
  const fields: Record<string, string> = {};
  if (Array.isArray(error?.fields)) {
    for (const item of error.fields as { field?: unknown; message?: unknown }[]) {
      // The first message for a field is the one to show.
      if (typeof item.field === "string" && typeof item.message === "string" && !(item.field in fields)) {
        fields[item.field] = item.message;
      }
    }
  }
  const retry = Number(response.headers.get("retry-after"));
  return {
    status: response.status,
    code: typeof error?.code === "string" ? error.code : "unknown",
    message: typeof error?.message === "string" ? error.message : "Something went wrong. Please try again.",
    fields,
    retryAfter: Number.isFinite(retry) && retry > 0 ? retry : null,
  };
}

export function createApi(options: ClientOptions = {}) {
  const baseUrl = options.baseUrl ?? "";
  const send = (input: Request) => (options.fetch ?? globalThis.fetch)(input);
  const withLock = options.withLock ?? webLock;
  const raceDelayMs = options.raceDelayMs ?? 150;

  let renewing: Promise<boolean> | null = null;

  /** Renews the session. `true` means the cookies are fresh, whoever renewed them. */
  function renewSession(): Promise<boolean> {
    renewing ??= withLock(async () => {
      const response = await send(new Request(`${baseUrl}/v1/auth/refresh`, { method: "POST" }));
      if (response.status === 204) return true;
      // Another tab got there first. Its new cookies are already ours.
      if (response.status === 401 && (await errorCode(response)) === "refresh_conflict") {
        await sleep(raceDelayMs);
        return true;
      }
      return false;
    }).finally(() => {
      renewing = null;
    });
    return renewing;
  }

  async function sendRenewing(request: Request): Promise<Response> {
    const again = request.clone();
    const response = await send(request);
    if (response.status !== 401 || NO_RENEWAL.test(new URL(request.url, "http://x").pathname)) return response;
    if ((await errorCode(response)) !== "unauthenticated") return response;
    return (await renewSession()) ? send(again) : response;
  }

  const client = createClient<paths>({ baseUrl, fetch: sendRenewing });

  async function call<D>(run: () => Promise<{ data?: D; error?: unknown; response: Response }>): Promise<Result<D>> {
    let out;
    try {
      out = await run();
    } catch {
      return { ok: false, error: NETWORK };
    }
    if (out.response.ok) return { ok: true, data: out.data as D };
    return { ok: false, error: await failureOf(out.response, out.error) };
  }

  return {
    auth: {
      register: (body: Schemas["RegisterRequest"]) => call(() => client.POST("/v1/auth/register", { body })),
      verifyEmail: (token: string) => call(() => client.POST("/v1/auth/verify-email", { body: { token } })),
      resendVerification: (email: string) => call(() => client.POST("/v1/auth/resend-verification", { body: { email } })),
      login: (body: Schemas["LoginRequest"]) => call(() => client.POST("/v1/auth/login", { body })),
      logout: () => call(() => client.POST("/v1/auth/logout")),
      forgotPassword: (email: string) => call(() => client.POST("/v1/auth/forgot-password", { body: { email } })),
      resetPassword: (token: string, newPassword: string) =>
        call(() => client.POST("/v1/auth/reset-password", { body: { token, new_password: newPassword } })),
      me: () => call(() => client.GET("/v1/auth/me")),
      /** Renames the account. */
      rename: (name: string) => call(() => client.PATCH("/v1/auth/me", { body: { name } })),
      /** Needs the password in use now. On success every session is over, this one included. */
      changePassword: (currentPassword: string, newPassword: string) =>
        call(() => client.POST("/v1/auth/change-password", { body: { current_password: currentPassword, new_password: newPassword } })),
    },
    /** What a signed-in person can see about their own account. */
    account: {
      storage: () => call(() => client.GET("/v1/me/storage")),
    },
    recordings: {
      list: (query: { cursor?: string; limit?: number; source?: string } = {}) => call(() => client.GET("/v1/recordings", { params: { query } })),
      get: (id: string) => call(() => client.GET("/v1/recordings/{id}", { params: { path: { id } } })),
      /** `version` is the one the caller read: if the recording has moved on, this fails with `version_conflict`. */
      rename: (id: string, version: number, title: string) =>
        call(() => client.PATCH("/v1/recordings/{id}", { params: { path: { id } }, body: { version, title } })),
      remove: (id: string) => call(() => client.DELETE("/v1/recordings/{id}", { params: { path: { id } } })),
      /** Opens or closes the conversation on a recording. */
      setComments: (id: string, version: number, enabled: boolean) =>
        call(() => client.PATCH("/v1/recordings/{id}", { params: { path: { id } }, body: { version, comments_enabled: enabled } })),
    },
    /** What anyone holding a share link can do: no account, no cookie. */
    share: {
      resolve: (code: string) => call(() => client.GET("/v1/s/{code}", { params: { path: { code } } })),
      play: (code: string, password?: string) =>
        call(() => client.POST("/v1/s/{code}/play", { params: { path: { code } }, body: password ? { password } : {} })),
      /** The conversation, newest first. `t` is the token of a viewing, for a link with a password. */
      comments: (code: string, query: { cursor?: string; limit?: number; t?: string } = {}) =>
        call(() => client.GET("/v1/s/{code}/comments", { params: { path: { code }, query } })),
      /** Leaves a comment or a reaction. Needs a signed-in account. */
      comment: (code: string, body: Schemas["CreateCommentRequest"], t?: string) =>
        call(() => client.POST("/v1/s/{code}/comments", { params: { path: { code }, query: t ? { t } : {} }, body })),
    },
    comments: {
      /** Removes one: its author may, and so may the owner of the recording. */
      remove: (id: string) => call(() => client.DELETE("/v1/comments/{id}", { params: { path: { id } } })),
    },
    links: {
      list: (recording: string) => call(() => client.GET("/v1/recordings/{id}/links", { params: { path: { id: recording } } })),
      create: (recording: string, body: Schemas["CreateLinkRequest"] = {}) =>
        call(() => client.POST("/v1/recordings/{id}/links", { params: { path: { id: recording } }, body })),
      revoke: (recording: string, link: string) =>
        call(() => client.DELETE("/v1/recordings/{id}/links/{link}", { params: { path: { id: recording, link } } })),
    },
  };
}

export type Api = ReturnType<typeof createApi>;

/** The app's client. */
export const api: Api = createApi();
