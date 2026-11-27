// What each role may do in a workspace, as the screens need to know it to offer only what will work. The API is
// the authority and enforces all of this itself (`crates/api/src/workspaces/permissions.rs`); a screen that
// offered more would only be told no.

import type { Schemas } from "./api/client";

export type Role = Schemas["WorkspaceRole"];

export const ROLE_LABEL: Record<Role, string> = { owner: "Owner", admin: "Admin", member: "Member" };

/** Renaming, seeing addresses and open invitations, inviting. */
export const canManage = (actor: Role): boolean => actor === "owner" || actor === "admin";

/** Ending the workspace, and choosing what the others are. */
export const isOwner = (actor: Role): boolean => actor === "owner";

/** What somebody may be invited as. Nobody is invited as an owner. */
export function inviteRoles(actor: Role): Role[] {
  if (actor === "owner") return ["member", "admin"];
  if (actor === "admin") return ["member"];
  return [];
}

/** Removing somebody else: an owner anyone, an admin members. (Leaving oneself is separate.) */
export function canRemove(actor: Role, target: Role): boolean {
  if (actor === "owner") return true;
  return actor === "admin" && target === "member";
}

/** Taking a recording out of the team: whoever made it, and the people who look after the workspace. */
export const canTakeOut = (actor: Role, mine: boolean): boolean => mine || canManage(actor);
