import { describe, expect, it } from "vitest";

import type { ApiFailure } from "@/lib/api/client";

import { initialPlayerState, nextPlayerState, pollDelaySeconds, preparingText, viewOf, type ShareInfo } from "./state";

const share = (over: Partial<ShareInfo>): ShareInfo => ({ code: "abcdefghijklmnopqrstuv", requires_password: false, ...over });
const failure = (code: string, over: Partial<ApiFailure> = {}): ApiFailure => ({ status: 400, code, message: "m", fields: {}, retryAfter: null, ...over });
const wording = (f: ApiFailure) => `wording for ${f.code}`;

describe("viewOf", () => {
  it("offers a player only for a video that is ready", () => {
    expect(viewOf(share({ title: "Week 3", status: "ready", duration_seconds: 125 }))).toEqual({ kind: "ready", title: "Week 3", durationSeconds: 125 });
    expect(viewOf(share({ title: "Week 3", status: "ready" }))).toMatchObject({ kind: "ready", durationSeconds: null });
  });

  it("says a video is being prepared, whichever stage it is at", () => {
    for (const status of ["capturing", "uploading", "processing"] as const) {
      expect(viewOf(share({ title: "Week 3", status }))).toEqual({ kind: "preparing", title: "Week 3", status });
    }
  });

  it("says when a video could not be processed", () => {
    expect(viewOf(share({ title: "Week 3", status: "failed" }))).toEqual({ kind: "failed", title: "Week 3" });
  });

  it("shows nothing about a protected link, even if the server said more", () => {
    expect(viewOf(share({ requires_password: true }))).toEqual({ kind: "protected" });
    expect(viewOf(share({ requires_password: true, title: "Secret", status: "ready" }))).toEqual({ kind: "protected" });
  });

  it("does not offer a player for a status it has not heard of", () => {
    expect(viewOf(share({ title: "Week 3", status: "hologram" as never }))).toMatchObject({ kind: "preparing" });
  });

  it("copes with a title that is missing", () => {
    expect(viewOf(share({ status: "ready" }))).toMatchObject({ title: "Untitled recording" });
  });
});

describe("pollDelaySeconds", () => {
  it("starts quick, slows down, and never goes beyond half a minute", () => {
    expect([0, 1, 2, 3, 4, 5, 10, 50].map(pollDelaySeconds)).toEqual([6, 9, 14, 20, 30, 30, 30, 30]);
    expect(pollDelaySeconds(-3)).toBe(6);
  });
});

describe("preparingText", () => {
  it("has words for every stage", () => {
    for (const status of ["capturing", "uploading", "processing"] as const) {
      const text = preparingText(status);
      expect(text.heading.length).toBeGreaterThan(3);
      expect(text.line).toMatch(/update by itself/);
    }
  });
});

describe("the player", () => {
  const started = { type: "started" as const, playlistUrl: "/v1/play/1/master.m3u8?t=x", expiresAt: "2026-11-21T10:00:00Z", watermark: "Shared view · ABCD1234" };

  it("goes from idle to starting to playing", () => {
    const starting = nextPlayerState(initialPlayerState, { type: "press" });
    expect(starting).toEqual({ phase: "starting" });
    expect(nextPlayerState(starting, started)).toEqual({ phase: "playing", playlistUrl: "/v1/play/1/master.m3u8?t=x", expiresAt: Date.parse("2026-11-21T10:00:00Z"), watermark: "Shared view · ABCD1234" });
  });

  it("ignores a second press while one is under way", () => {
    const starting = nextPlayerState(initialPlayerState, { type: "press" });
    expect(nextPlayerState(starting, { type: "press" })).toBe(starting);
  });

  it("asks for the password, and says when it was wrong", () => {
    const starting = nextPlayerState(initialPlayerState, { type: "press" });
    expect(nextPlayerState(starting, { type: "refused", failure: failure("password_required"), retryWording: wording })).toMatchObject({ phase: "idle", passwordProblem: "Enter the password to watch." });
    expect(nextPlayerState(starting, { type: "refused", failure: failure("wrong_password"), retryWording: wording })).toMatchObject({ phase: "idle", passwordProblem: "That password isn't right." });
  });

  it("says the video is not ready, and the link is closed", () => {
    expect(nextPlayerState({ phase: "starting" }, { type: "refused", failure: failure("not_ready"), retryWording: wording })).toEqual({ phase: "preparing" });
    expect(nextPlayerState({ phase: "starting" }, { type: "refused", failure: failure("link_unavailable"), retryWording: wording })).toEqual({ phase: "closed" });
    expect(nextPlayerState({ phase: "starting" }, { type: "refused", failure: failure("not_found"), retryWording: wording })).toEqual({ phase: "closed" });
  });

  it("puts any other refusal into words and goes back to the start, so pressing again is possible", () => {
    expect(nextPlayerState({ phase: "starting" }, { type: "refused", failure: failure("too_many_requests", { retryAfter: 60 }), retryWording: wording })).toEqual({ phase: "idle", notice: "wording for too_many_requests", passwordProblem: null });
    expect(nextPlayerState({ phase: "starting" }, { type: "refused", failure: failure("network", { status: 0 }), retryWording: wording })).toMatchObject({ phase: "idle" });
  });

  it("returns to the start with an explanation when a viewing ends", () => {
    const playing = nextPlayerState({ phase: "starting" }, started);
    expect(nextPlayerState(playing, { type: "ended", notice: "This viewing has ended." })).toEqual({ phase: "idle", notice: "This viewing has ended.", passwordProblem: null });
  });
});
