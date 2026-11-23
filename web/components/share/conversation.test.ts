import { describe, expect, it } from "vitest";

import { EMOJIS, ago, atLabel, marks, mergeComments, newestFirst, position, refreshFirstPage, refusal, removeComment, type Comment } from "./conversation";

function made(id: string, createdAt: string, extra: Partial<Comment> = {}): Comment {
  return { id, kind: "comment", body: id, emoji: null, at_ms: null, author: { name: "A" }, created_at: createdAt, mine: false, can_delete: false, ...extra };
}

const t = (minute: number) => `2026-11-23T10:${String(minute).padStart(2, "0")}:00Z`;

describe("the order of a conversation", () => {
  it("is newest first, and the id settles a tie", () => {
    const rows = [made("a", t(1)), made("c", t(3)), made("b", t(2)), made("z", t(3))];
    expect([...rows].sort(newestFirst).map((r) => r.id)).toEqual(["z", "c", "b", "a"]);
  });
});

describe("mergeComments", () => {
  it("shows a comment once, as the newer copy says, in order", () => {
    const shown = [made("b", t(2)), made("a", t(1))];
    const merged = mergeComments(shown, [made("c", t(3)), made("b", t(2), { body: "edited", can_delete: true })]);
    expect(merged.map((r) => r.id)).toEqual(["c", "b", "a"]);
    expect(merged[1]).toMatchObject({ body: "edited", can_delete: true });
  });
});

describe("refreshFirstPage", () => {
  const shown = [made("e", t(5)), made("d", t(4)), made("c", t(3)), made("b", t(2)), made("a", t(1))];

  it("takes in what is new and drops what was removed within the first page, and keeps the older pages", () => {
    // Two pages are shown (e d | c b a). The first page is now: f e c (d was removed, f is new): c is the oldest of it.
    const fresh = [made("f", t(6)), made("e", t(5)), made("c", t(3))];
    expect(refreshFirstPage(shown, fresh, true).map((r) => r.id)).toEqual(["f", "e", "c", "b", "a"]);
  });

  it("is the whole conversation when there is no more, so anything older that was shown is gone too", () => {
    expect(refreshFirstPage(shown, [made("e", t(5))], false).map((r) => r.id)).toEqual(["e"]);
  });

  it("empties when everything was removed", () => {
    expect(refreshFirstPage(shown, [], false)).toEqual([]);
    expect(refreshFirstPage(shown, [], true)).toEqual([]);
  });
});

describe("removeComment", () => {
  it("takes one out and leaves the rest", () => {
    expect(removeComment([made("a", t(1)), made("b", t(2))], "a").map((r) => r.id)).toEqual(["b"]);
  });
});

describe("position and marks", () => {
  it("places a comment along the video, and not at all when it is about the whole video or the length is unknown", () => {
    expect(position({ at_ms: 30_000 }, 60_000)).toBe(0.5);
    expect(position({ at_ms: 90_000 }, 60_000)).toBe(1);
    expect(position({ at_ms: null }, 60_000)).toBeNull();
    expect(position({ at_ms: 5 }, null)).toBeNull();
    expect(position({ at_ms: 5 }, 0)).toBeNull();
  });

  it("groups what is close together into one mark with a count, the reactions most used first", () => {
    const rows = [
      made("1", t(1), { kind: "reaction", body: null, emoji: "🔥", at_ms: 10_000 }),
      made("2", t(2), { kind: "reaction", body: null, emoji: "🔥", at_ms: 10_200 }),
      made("3", t(3), { kind: "reaction", body: null, emoji: "👏", at_ms: 10_100 }),
      made("4", t(4), { at_ms: 10_300 }),
      made("5", t(5), { at_ms: 50_000 }),
      made("6", t(6)),
    ];
    const out = marks(rows, 100_000);
    expect(out).toHaveLength(2);
    expect(out[0]).toMatchObject({ count: 4, emojis: ["🔥", "👏"], atMs: 10_000 });
    expect(out[1]).toMatchObject({ count: 1, emojis: [], atMs: 50_000 });
    expect(out[0]!.at).toBeLessThan(out[1]!.at);
    expect(marks(rows, null)).toEqual([]);
    expect(marks([], 100_000)).toEqual([]);
  });

  it("keeps the last moment of the video on the timeline", () => {
    const out = marks([made("x", t(1), { at_ms: 100_000 })], 100_000, 10);
    expect(out[0]!.at).toBeCloseTo(0.95);
  });
});

describe("words for people", () => {
  it("says a moment as a clock", () => {
    expect(atLabel(0)).toBe("0:00");
    expect(atLabel(83_900)).toBe("1:23");
    expect(atLabel(3_723_000)).toBe("1:02:03");
    expect(atLabel(-5)).toBe("0:00");
  });

  it("says how long ago, never in the future", () => {
    const now = Date.parse("2026-11-23T12:00:00Z");
    expect(ago("2026-11-23T11:59:50Z", now)).toBe("just now");
    expect(ago("2026-11-23T12:00:30Z", now)).toBe("just now");
    expect(ago("2026-11-23T11:59:00Z", now)).toBe("1 minute ago");
    expect(ago("2026-11-23T11:30:00Z", now)).toBe("30 minutes ago");
    expect(ago("2026-11-23T09:00:00Z", now)).toBe("3 hours ago");
    expect(ago("2026-11-21T12:00:00Z", now)).toBe("2 days ago");
    expect(ago("2026-10-01T12:00:00Z", now)).toMatch(/2026/);
  });

  it("explains a refusal by what it was, not by a status number", () => {
    const failure = (code: string, status = 409, retryAfter: number | null = null) => ({ status, code, message: "server said", retryAfter });
    expect(refusal(failure("comments_off"))).toMatch(/turned comments off/);
    expect(refusal(failure("too_many_comments"))).toMatch(/as one person can/);
    expect(refusal(failure("too_many_requests", 429, 8))).toBe("Slow down a little. Try again in 8 seconds.");
    expect(refusal(failure("too_many_requests", 429, 1))).toBe("Slow down a little. Try again in 1 second.");
    expect(refusal(failure("validation_failed", 422))).toBe("server said");
    expect(refusal(failure("anything_else", 500))).toBe("That did not go through. Please try again.");
  });

  it("offers six reactions", () => {
    expect(EMOJIS).toHaveLength(6);
  });
});
