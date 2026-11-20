import { describe, expect, it } from "vitest";

import type { ApiFailure } from "@/lib/api/client";

import { describeFailure, waitFor } from "./messages";

const failure = (over: Partial<ApiFailure>): ApiFailure => ({
  status: 400,
  code: "x",
  message: "From the server.",
  fields: {},
  retryAfter: null,
  ...over,
});

describe("waitFor", () => {
  it("speaks in the unit a person would", () => {
    expect(waitFor(1)).toBe("1 second");
    expect(waitFor(45)).toBe("45 seconds");
    expect(waitFor(60)).toBe("1 minute");
    expect(waitFor(61)).toBe("2 minutes");
    expect(waitFor(3599)).toBe("60 minutes");
    expect(waitFor(3600)).toBe("1 hour");
    expect(waitFor(7300)).toBe("3 hours");
  });

  it("never promises sooner than the server said", () => {
    for (const seconds of [1, 59, 61, 119, 120, 3601]) {
      const text = waitFor(seconds);
      const [amount, unit] = text.split(" ");
      const given = Number(amount) * (unit.startsWith("second") ? 1 : unit.startsWith("minute") ? 60 : 3600);
      expect(given).toBeGreaterThanOrEqual(seconds);
    }
  });

  it("says something sensible when the server did not say", () => {
    expect(waitFor(null)).toBe("a little while");
    expect(waitFor(0)).toBe("a little while");
  });
});

describe("describeFailure", () => {
  it("turns a limit into a wait", () => {
    expect(describeFailure(failure({ status: 429, code: "too_many_requests", retryAfter: 600 }))).toBe(
      "Too many attempts. Try again in 10 minutes.",
    );
  });

  it("does not pass our own faults off as the person's", () => {
    expect(describeFailure(failure({ status: 500, code: "internal_error", message: "Something went wrong. Please try again." }))).toMatch(/our side/);
    expect(describeFailure(failure({ status: 503, code: "unavailable" }))).toMatch(/busy/);
  });

  it("keeps the server's message when it is already good", () => {
    expect(describeFailure(failure({ code: "invalid_token", message: "This link is invalid or has expired. Request a new one." }))).toBe(
      "This link is invalid or has expired. Request a new one.",
    );
    expect(describeFailure(failure({ status: 0, code: "network", message: "We couldn't reach Zeno." }))).toBe("We couldn't reach Zeno.");
  });
});
