import { describe, expect, it } from "vitest";

import { canManage, canRemove, inviteRoles, isOwner } from "./team";

const ROLES = ["owner", "admin", "member"] as const;

describe("what each role may do", () => {
  it("lets owners and admins manage people, and only owners choose roles or end the workspace", () => {
    expect(ROLES.map(canManage)).toEqual([true, true, false]);
    expect(ROLES.map(isOwner)).toEqual([true, false, false]);
  });

  it("invites nobody as an owner, and an admin invites only members", () => {
    expect(inviteRoles("owner")).toEqual(["member", "admin"]);
    expect(inviteRoles("admin")).toEqual(["member"]);
    expect(inviteRoles("member")).toEqual([]);
  });

  it("lets an owner remove anyone and an admin remove members", () => {
    for (const target of ROLES) {
      expect(canRemove("owner", target)).toBe(true);
      expect(canRemove("admin", target)).toBe(target === "member");
      expect(canRemove("member", target)).toBe(false);
    }
  });
});
