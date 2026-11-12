import { describe, expect, it } from "vitest";

import en from "@/messages/en.json";

import { ApiError, NetworkError } from "./errors";
import { errorMessage } from "./messages";

const api = (status: number, code: string, retryAfterSeconds?: number) =>
  new ApiError({ status, code, message: "English prose that must never be shown", retryAfterSeconds });

describe("what a failure is explained as", () => {
  it("picks the message by code, never by the server's words", () => {
    expect(errorMessage(api(401, "invalid_credentials"))).toEqual({ key: "invalid_credentials" });
    expect(errorMessage(api(403, "email_not_verified"))).toEqual({ key: "email_not_verified" });
    expect(errorMessage(api(422, "validation_failed"))).toEqual({ key: "validation_failed" });
  });

  it("tells a rate-limited person how long to wait, when the server said", () => {
    expect(errorMessage(api(429, "too_many_requests", 42))).toEqual({
      key: "too_many_requests",
      values: { seconds: 42 },
    });
    expect(errorMessage(api(429, "too_many_requests"))).toEqual({ key: "too_many_requests_soon" });
  });

  it("treats any other server failure as our fault, and anything unknown as unknown", () => {
    expect(errorMessage(api(500, "something_new"))).toEqual({ key: "internal_error" });
    expect(errorMessage(api(502, "http_502"))).toEqual({ key: "internal_error" });
    expect(errorMessage(api(409, "something_new"))).toEqual({ key: "unknown" });
    expect(errorMessage(new Error("boom"))).toEqual({ key: "unknown" });
    expect(errorMessage("a string")).toEqual({ key: "unknown" });
  });

  it("explains a request that never got an answer by why", () => {
    expect(errorMessage(new NetworkError("offline", "x"))).toEqual({ key: "offline" });
    expect(errorMessage(new NetworkError("timeout", "x"))).toEqual({ key: "timeout" });
    expect(errorMessage(new NetworkError("aborted", "x"))).toEqual({ key: "aborted" });
  });

  it("only ever names a message that exists", () => {
    const known = Object.keys(en.errors);
    const failures = [
      api(401, "invalid_credentials"),
      api(401, "unauthenticated"),
      api(401, "session_expired"),
      api(413, "payload_too_large"),
      api(429, "too_many_requests", 3),
      api(429, "too_many_requests"),
      api(500, "internal_error"),
      api(409, "x"),
      new NetworkError("offline", ""),
      new NetworkError("timeout", ""),
      new NetworkError("aborted", ""),
      new Error("x"),
    ];
    for (const failure of failures) expect(known).toContain(errorMessage(failure).key);
  });
});
