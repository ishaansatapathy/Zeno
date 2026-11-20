import { describe, expect, it } from "vitest";

import { createApi } from "./client";

const BASE = "http://api.test";

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });

const failure = (status: number, code: string, message = "m") => json(status, { error: { code, message } });

const USER = { user: { id: "9f5c1d0e-0000-4000-8000-000000000001", name: "Asha", email: "a@x.in", role: "creator", email_verified: true } };

/** A fake server: answers by `METHOD /path`, remembering each call. */
function server(routes: Record<string, (call: number, request: Request) => Response | Promise<Response>>) {
  const calls: Record<string, number> = {};
  const log: string[] = [];
  const fetcher = async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init);
    const key = `${request.method} ${new URL(request.url).pathname}`;
    calls[key] = (calls[key] ?? 0) + 1;
    log.push(key);
    const route = routes[key];
    if (!route) throw new Error(`unexpected call: ${key}`);
    return route(calls[key], request);
  };
  return { fetcher: fetcher as typeof fetch, calls, log };
}

const api = (fake: ReturnType<typeof server>, extra = {}) =>
  createApi({ baseUrl: BASE, fetch: fake.fetcher, withLock: (task) => task(), raceDelayMs: 0, ...extra });

describe("results", () => {
  it("returns the typed body on success", async () => {
    const fake = server({ "POST /v1/auth/login": () => json(200, USER) });
    const result = await api(fake).auth.login({ email: "a@x.in", password: "pw" });
    expect(result).toEqual({ ok: true, data: USER });
  });

  it("sends the JSON body the types describe", async () => {
    let seen: unknown;
    const fake = server({
      "POST /v1/auth/reset-password": async (_n, request) => {
        seen = await request.json();
        return json(200, { message: "ok" });
      },
    });
    await api(fake).auth.resetPassword("tok", "a long new password");
    expect(seen).toEqual({ token: "tok", new_password: "a long new password" });
  });

  it("maps field errors, keeping the first message for each field", async () => {
    const fake = server({
      "POST /v1/auth/register": () =>
        json(422, {
          error: {
            code: "validation_failed",
            message: "Some fields are invalid.",
            fields: [
              { field: "email", message: "Enter a valid email address." },
              { field: "password", message: "Too short." },
              { field: "password", message: "Too common." },
            ],
          },
        }),
    });
    const result = await api(fake).auth.register({ name: "A", email: "x", password: "y" });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error.status).toBe(422);
    expect(result.error.code).toBe("validation_failed");
    expect(result.error.fields).toEqual({ email: "Enter a valid email address.", password: "Too short." });
  });

  it("reads how long to wait from Retry-After", async () => {
    const fake = server({
      "POST /v1/auth/login": () => json(429, { error: { code: "too_many_requests", message: "Slow down." } }, { "retry-after": "120" }),
    });
    const result = await api(fake).auth.login({ email: "a@x.in", password: "pw" });
    expect(result.ok === false && result.error.retryAfter).toBe(120);
  });

  it("treats a server that cannot be reached as a failure, not an exception", async () => {
    const down = (() => Promise.reject(new TypeError("fetch failed"))) as typeof fetch;
    const result = await createApi({ baseUrl: BASE, fetch: down }).auth.me();
    expect(result).toMatchObject({ ok: false, error: { status: 0, code: "network" } });
  });

  it("survives an error body that is not JSON", async () => {
    const fake = server({ "POST /v1/auth/login": () => new Response("Bad gateway", { status: 502 }) });
    const result = await api(fake).auth.login({ email: "a@x.in", password: "pw" });
    expect(result).toMatchObject({ ok: false, error: { status: 502, code: "unknown" } });
  });

  it("succeeds on a 204 with no body", async () => {
    const fake = server({ "POST /v1/auth/logout": () => new Response(null, { status: 204 }) });
    expect((await api(fake).auth.logout()).ok).toBe(true);
  });
});

describe("renewing the session", () => {
  it("renews once and repeats the request when the session has lapsed", async () => {
    const fake = server({
      "GET /v1/auth/me": (n) => (n === 1 ? failure(401, "unauthenticated") : json(200, USER)),
      "POST /v1/auth/refresh": () => new Response(null, { status: 204 }),
    });
    const result = await api(fake).auth.me();
    expect(result).toEqual({ ok: true, data: USER });
    expect(fake.log).toEqual(["GET /v1/auth/me", "POST /v1/auth/refresh", "GET /v1/auth/me"]);
  });

  it("renews once for any number of requests that lapse together", async () => {
    let renewed = false;
    const fake = server({
      "GET /v1/auth/me": () => (renewed ? json(200, USER) : failure(401, "unauthenticated")),
      "POST /v1/auth/refresh": async () => {
        await new Promise((resolve) => setTimeout(resolve, 20));
        renewed = true;
        return new Response(null, { status: 204 });
      },
    });
    const client = api(fake);
    const results = await Promise.all([client.auth.me(), client.auth.me(), client.auth.me(), client.auth.me()]);
    expect(results.every((r) => r.ok)).toBe(true);
    expect(fake.calls["POST /v1/auth/refresh"]).toBe(1);
  });

  it("gives up, once, when the session cannot be renewed", async () => {
    const fake = server({
      "GET /v1/auth/me": () => failure(401, "unauthenticated"),
      "POST /v1/auth/refresh": () => failure(401, "session_expired"),
    });
    const result = await api(fake).auth.me();
    expect(result).toMatchObject({ ok: false, error: { status: 401, code: "unauthenticated" } });
    expect(fake.calls["GET /v1/auth/me"]).toBe(1);
    expect(fake.calls["POST /v1/auth/refresh"]).toBe(1);
  });

  it("repeats the request, not the refresh, when another tab won the race", async () => {
    const fake = server({
      "GET /v1/auth/me": (n) => (n === 1 ? failure(401, "unauthenticated") : json(200, USER)),
      "POST /v1/auth/refresh": () => failure(401, "refresh_conflict"),
    });
    expect((await api(fake).auth.me()).ok).toBe(true);
    expect(fake.calls["POST /v1/auth/refresh"]).toBe(1);
  });

  it("does not try to renew a wrong password, or anything else that is not a lapsed session", async () => {
    const fake = server({ "POST /v1/auth/login": () => failure(401, "invalid_credentials") });
    const result = await api(fake).auth.login({ email: "a@x.in", password: "nope" });
    expect(result).toMatchObject({ ok: false, error: { code: "invalid_credentials" } });
    expect(fake.log).toEqual(["POST /v1/auth/login"]);
  });

  it("does not renew a session that has ended", async () => {
    const fake = server({ "GET /v1/auth/me": () => failure(401, "session_expired") });
    expect((await api(fake).auth.me()).ok).toBe(false);
    expect(fake.log).toEqual(["GET /v1/auth/me"]);
  });

  it("renews one tab at a time", async () => {
    // Two clients stand in for two tabs; one lock, as the browser has one per origin.
    let held = false;
    let overlap = 0;
    let tail: Promise<unknown> = Promise.resolve();
    const lock = <T,>(task: () => Promise<T>): Promise<T> => {
      const run = tail.then(async () => {
        if (held) overlap += 1;
        held = true;
        try {
          return await task();
        } finally {
          held = false;
        }
      });
      tail = run.catch(() => undefined);
      return run;
    };
    let fresh = false;
    const fake = server({
      "GET /v1/auth/me": () => (fresh ? json(200, USER) : failure(401, "unauthenticated")),
      "POST /v1/auth/refresh": async () => {
        await new Promise((resolve) => setTimeout(resolve, 15));
        fresh = true;
        return new Response(null, { status: 204 });
      },
    });
    const [a, b] = [api(fake, { withLock: lock }), api(fake, { withLock: lock })];
    const results = await Promise.all([a.auth.me(), b.auth.me()]);
    expect(results.every((r) => r.ok)).toBe(true);
    expect(overlap).toBe(0);
  });
});
