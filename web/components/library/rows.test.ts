import { describe, expect, it } from "vitest";

import { addPage, clock, newestFirst, plainLink, removeRow, replaceRow, sourceName, statusName, type Link, type Recording } from "./rows";

const row = (n: number, over: Partial<Recording> = {}): Recording => ({
  id: `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`,
  owner_id: "00000000-0000-4000-8000-0000000000aa",
  title: `Recording ${n}`,
  source: "extension",
  visibility: "private",
  comments_enabled: true,
  status: "ready",
  video_id: null,
  duration_seconds: null,
  version: 1,
  view_count: 0,
  created_at: `2026-11-${String(10 + n).padStart(2, "0")}T10:00:00Z`,
  updated_at: `2026-11-${String(10 + n).padStart(2, "0")}T10:00:00Z`,
  ...over,
});

const link = (over: Partial<Link>): Link => ({
  id: "11111111-1111-4111-8111-111111111111",
  code: "abcdefghijklmnopqrstuv",
  url: "http://localhost:3000/s/abcdefghijklmnopqrstuv",
  has_password: false,
  state: "active",
  expires_at: null,
  revoked_at: null,
  created_at: "2026-11-20T10:00:00Z",
  ...over,
});

describe("addPage", () => {
  it("keeps the library newest first as pages arrive", () => {
    const first = [row(5), row(4)];
    const second = [row(3), row(2)];
    expect(addPage(first, second).map((r) => r.title)).toEqual(["Recording 5", "Recording 4", "Recording 3", "Recording 2"]);
  });

  it("shows a recording once, as the newer copy says, if two pages both held it", () => {
    const merged = addPage([row(5), row(4)], [row(4, { title: "Renamed elsewhere", version: 2 }), row(3)]);
    expect(merged).toHaveLength(3);
    expect(merged.find((r) => r.id === row(4).id)).toMatchObject({ title: "Renamed elsewhere", version: 2 });
  });

  it("breaks a tie between equal times by id, as the API does", () => {
    const a = row(1, { created_at: "2026-11-20T10:00:00Z" });
    const b = row(2, { created_at: "2026-11-20T10:00:00Z" });
    expect([a, b].sort(newestFirst).map((r) => r.id)).toEqual([b.id, a.id]);
  });
});

describe("replaceRow and removeRow", () => {
  it("swaps one row and leaves the rest alone", () => {
    const rows = [row(3), row(2), row(1)];
    const next = replaceRow(rows, row(2, { title: "New", version: 2 }));
    expect(next.map((r) => r.title)).toEqual(["Recording 3", "New", "Recording 1"]);
    expect(rows[1].title).toBe("Recording 2");
  });

  it("does not add a recording that was not listed", () => {
    expect(replaceRow([row(1)], row(9))).toEqual([row(1)]);
  });

  it("drops one row", () => {
    expect(removeRow([row(2), row(1)], row(2).id).map((r) => r.id)).toEqual([row(1).id]);
    expect(removeRow([row(1)], row(9).id)).toHaveLength(1);
  });
});

describe("plainLink", () => {
  it("is the link that works for anyone", () => {
    expect(plainLink([link({ state: "revoked" }), link({ id: "b", has_password: true }), link({ id: "c" })])?.id).toBe("c");
  });

  it("is nothing when every link has conditions or has ended", () => {
    expect(
      plainLink([
        link({ state: "expired" }),
        link({ state: "revoked", revoked_at: "2026-11-20T11:00:00Z" }),
        link({ has_password: true }),
        link({ expires_at: "2027-01-01T00:00:00Z" }),
      ]),
    ).toBeUndefined();
    expect(plainLink([])).toBeUndefined();
  });
});

describe("words", () => {
  it("names what the API says, and shows the API's word for anything new", () => {
    expect(sourceName("extension")).toBe("Extension");
    expect(statusName("capturing")).toBe("Capturing");
    expect(sourceName("hologram")).toBe("hologram");
  });

  it("writes a length as a clock", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(83)).toBe("1:23");
    expect(clock(59.6)).toBe("1:00");
    expect(clock(3723)).toBe("1:02:03");
  });
});
