/* eslint-disable @typescript-eslint/no-explicit-any -- a failure is inspected by whatever shape it has */
import { describe, expect, it, vi } from "vitest";

import { createClient } from "./client";
import { ApiError, NetworkError, fieldMessages, isApiError } from "./errors";

type Call = { url: string; init: RequestInit };

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });

const unauthorized = () =>
  json(401, { error: { code: "unauthorized", message: "Sign in to continue." } });

/** A fake network: `route` decides each answer, and every request is recorded. */
function network(route: (call: Call, n: number) => Response | Promise<Response>) {
  const calls: Call[] = [];
  const fetchFn = (async (url: string, init: RequestInit) => {
    const call = { url, init };
    calls.push(call);
    return route(call, calls.length);
  }) as unknown as typeof fetch;
  return { calls, fetchFn };
}

/** The client, with answers typed loosely: these tests look at whatever shape comes back. */
type Loose = Record<"get" | "post" | "put" | "patch" | "delete", (path: string, opts?: any) => Promise<any>>;

const client = (fetchFn: typeof fetch, extra: Partial<Parameters<typeof createClient>[0]> = {}) =>
  createClient({ baseUrl: "https://api.example", fetch: fetchFn, requestId: () => "req-1", ...extra }) as unknown as Loose;

const refreshes = (calls: Call[]) => calls.filter((c) => c.url.endsWith("/v1/auth/refresh")).length;

describe("what a request looks like", () => {
  it("sends the session cookies, a request id, and a JSON body only when there is one", async () => {
    const net = network(() => json(200, { ok: true }));
    const api = client(net.fetchFn);

    await api.get("/v1/me/earnings");
    await api.post("/v1/me/payouts", { body: { amount_paise: 50_000 } });

    const [get, post] = net.calls;
    expect(get.init.credentials).toBe("include");
    expect(get.init.method).toBe("GET");
    expect(get.init.body).toBeUndefined();
    expect((get.init.headers as Record<string, string>)["content-type"]).toBeUndefined();
    expect((get.init.headers as Record<string, string>)["x-request-id"]).toBe("req-1");
    expect(post.init.credentials).toBe("include");
    expect((post.init.headers as Record<string, string>)["content-type"]).toBe("application/json");
    expect(post.init.body).toBe('{"amount_paise":50000}');
  });

  it("encodes the query, leaves out what is undefined, and sends an idempotency key", async () => {
    const net = network(() => json(200, {}));
    const api = client(net.fetchFn);

    await api.get("/v1/me/earnings/entries", {
      query: { cursor: "a b&c=d", limit: 20, empty: undefined, none: null, flag: false },
    });
    await api.post("/v1/checkout/abc", { idempotencyKey: "key-1" });

    expect(net.calls[0].url).toBe(
      "https://api.example/v1/me/earnings/entries?cursor=a+b%26c%3Dd&limit=20&flag=false",
    );
    expect((net.calls[1].init.headers as Record<string, string>)["idempotency-key"]).toBe("key-1");
  });

  it("returns nothing for a 204 and refuses an answer that is not JSON", async () => {
    const net = network((_, n) => (n === 1 ? new Response(null, { status: 204 }) : new Response("<html>", { status: 200 })));
    const api = client(net.fetchFn);

    await expect(api.post("/v1/auth/logout")).resolves.toBeUndefined();
    await expect(api.get("/v1/me")).rejects.toMatchObject({ code: "invalid_response" });
  });
});

describe("what an error looks like", () => {
  it("maps the API's error body, with the fields and the request id", async () => {
    const net = network(() =>
      json(422, {
        error: {
          code: "validation_failed",
          message: "Some fields are invalid.",
          fields: [
            { field: "email", message: "Enter a valid email." },
            { field: "email", message: "A second message for the same field." },
            { field: "password", message: "Too short." },
            { nonsense: true },
          ],
        },
      }),
    );
    const error = await client(net.fetchFn).post("/v1/auth/register", { body: {} }).catch((e: any) => e);

    expect(error).toBeInstanceOf(ApiError);
    expect(isApiError(error, "validation_failed")).toBe(true);
    expect(error).toMatchObject({ status: 422, requestId: "req-1" });
    expect(error.fields).toHaveLength(3);
    expect(fieldMessages(error)).toEqual({ email: "Enter a valid email.", password: "Too short." });
  });

  it("copes with an answer that is not the API's (a proxy's page, an empty body)", async () => {
    for (const body of ["<html>Bad gateway</html>", "", "null", '{"error":"nope"}']) {
      const net = network(() => new Response(body, { status: 502 }));
      const error = await client(net.fetchFn).get("/v1/me").catch((e: any) => e);
      expect(error).toMatchObject({ status: 502, code: "http_502" });
      expect(error.fields).toEqual([]);
    }
  });

  it("reads Retry-After from a 429, and ignores one that is nonsense", async () => {
    const limited = (after: string) => network(() => json(429, { error: { code: "too_many_requests", message: "Slow down." } }, { "retry-after": after }));
    expect(await client(limited("42").fetchFn).get("/v1/x").catch((e: any) => e)).toMatchObject({ code: "too_many_requests", retryAfterSeconds: 42 });
    expect(await client(limited("soon").fetchFn).get("/v1/x").catch((e: any) => e)).toMatchObject({ retryAfterSeconds: undefined });
    expect(await client(limited("-5").fetchFn).get("/v1/x").catch((e: any) => e)).toMatchObject({ retryAfterSeconds: undefined });
  });
});

describe("a session that expires", () => {
  it("is renewed once and the call repeated", async () => {
    let renewed = false;
    const net = network((call) => {
      if (call.url.endsWith("/v1/auth/refresh")) {
        renewed = true;
        return json(200, {});
      }
      return renewed ? json(200, { earnings: 1 }) : unauthorized();
    });

    const result = await client(net.fetchFn).post("/v1/me/payouts", { body: { amount_paise: 100 } });

    expect(result).toEqual({ earnings: 1 });
    expect(net.calls.map((c) => c.url.replace("https://api.example", ""))).toEqual([
      "/v1/me/payouts",
      "/v1/auth/refresh",
      "/v1/me/payouts",
    ]);
    // The repeat sends exactly what the first try did.
    expect(net.calls[2].init.body).toBe(net.calls[0].init.body);
  });

  it("makes one refresh however many calls failed at once", async () => {
    let renewed = false;
    const net = network(async (call) => {
      if (call.url.endsWith("/v1/auth/refresh")) {
        await new Promise((resolve) => setTimeout(resolve, 20));
        renewed = true;
        return json(200, {});
      }
      return renewed ? json(200, { ok: call.url }) : unauthorized();
    });
    const api = client(net.fetchFn);

    const results = await Promise.all(Array.from({ length: 10 }, (_, i) => api.get(`/v1/thing/${i}`)));

    expect(results).toHaveLength(10);
    expect(refreshes(net.calls)).toBe(1);
    // Ten that failed, one refresh, ten that were repeated.
    expect(net.calls).toHaveLength(21);
  });

  it("can be renewed again the next time it expires", async () => {
    let valid = false;
    const net = network((call) => {
      if (call.url.endsWith("/v1/auth/refresh")) {
        valid = true;
        return json(200, {});
      }
      return valid ? json(200, {}) : unauthorized();
    });
    const api = client(net.fetchFn);

    await api.get("/v1/a");
    expect(refreshes(net.calls)).toBe(1);
    await api.get("/v1/b"); // still valid: no refresh
    expect(refreshes(net.calls)).toBe(1);
    valid = false; // the access cookie runs out again
    await api.get("/v1/c");
    expect(refreshes(net.calls)).toBe(2);
  });

  it("ends the session, once, when it cannot be renewed", async () => {
    const onSessionExpired = vi.fn();
    const net = network(() => unauthorized());
    const api = client(net.fetchFn, { onSessionExpired });

    const results = await Promise.allSettled(Array.from({ length: 10 }, (_, i) => api.get(`/v1/thing/${i}`)));

    for (const result of results) {
      expect(result.status).toBe("rejected");
      expect((result as PromiseRejectedResult).reason).toMatchObject({ status: 401, code: "unauthorized" });
    }
    expect(refreshes(net.calls)).toBe(1);
    // Ten calls failed together; that is one sign-out.
    expect(onSessionExpired).toHaveBeenCalledTimes(1);
    // Nothing was repeated after the refresh failed.
    expect(net.calls).toHaveLength(11);
  });

  it("repeats a call only once, and does not refresh again when the repeat is refused too", async () => {
    const net = network((call) => (call.url.endsWith("/v1/auth/refresh") ? json(200, {}) : unauthorized()));

    await expect(client(net.fetchFn).get("/v1/me/earnings")).rejects.toMatchObject({ status: 401 });

    expect(refreshes(net.calls)).toBe(1);
    expect(net.calls).toHaveLength(3);
  });

  it("does not mistake a wrong password for an expired session", async () => {
    for (const path of ["/v1/auth/login", "/v1/auth/register", "/v1/auth/reset-password", "/v1/auth/refresh", "/v1/auth/logout"]) {
      const onSessionExpired = vi.fn();
      const net = network(() => json(401, { error: { code: "invalid_credentials", message: "No." } }));

      await expect(client(net.fetchFn, { onSessionExpired }).post(path, { body: {} })).rejects.toMatchObject({ status: 401 });

      expect(net.calls, path).toHaveLength(1);
      expect(onSessionExpired, path).not.toHaveBeenCalled();
    }
  });

  it("does not call a dead network a signed-out user", async () => {
    const onSessionExpired = vi.fn();
    const net = network((call) => {
      if (call.url.endsWith("/v1/auth/refresh")) throw new TypeError("Failed to fetch");
      return unauthorized();
    });

    const error = await client(net.fetchFn, { onSessionExpired }).get("/v1/me").catch((e: any) => e);

    expect(error).toBeInstanceOf(NetworkError);
    expect(error.reason).toBe("offline");
    expect(onSessionExpired).not.toHaveBeenCalled();
  });
});

describe("a request that never gets an answer", () => {
  it("times out", async () => {
    const net = network(
      (call) =>
        new Promise<Response>((_, reject) => {
          (call.init.signal as AbortSignal).addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );

    const error = await client(net.fetchFn, { timeoutMs: 30 }).get("/v1/slow").catch((e: any) => e);

    expect(error).toBeInstanceOf(NetworkError);
    expect(error.reason).toBe("timeout");
  });

  it("stops when the caller gives up, and says so", async () => {
    const net = network(
      (call) =>
        new Promise<Response>((_, reject) => {
          (call.init.signal as AbortSignal).addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );
    const controller = new AbortController();
    const pending = client(net.fetchFn).get("/v1/slow", { signal: controller.signal }).catch((e: any) => e);
    controller.abort();

    expect((await pending).reason).toBe("aborted");
    // A signal that is already cancelled gives the same answer.
    const early = await client(net.fetchFn).get("/v1/slow", { signal: controller.signal }).catch((e: any) => e);
    expect(early.reason).toBe("aborted");
  });

  it("reports a failed connection as offline", async () => {
    const net = network(() => {
      throw new TypeError("Failed to fetch");
    });
    const error = await client(net.fetchFn).get("/v1/me").catch((e: any) => e);
    expect(error).toMatchObject({ name: "NetworkError", reason: "offline" });
  });
});
