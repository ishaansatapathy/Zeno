//! Who may do what in a workspace. The rules are here and nowhere else, as plain functions of the roles
//! involved, so the whole table is tested without a database.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a person is in a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type, Serialize, Deserialize, ToSchema)]
#[sqlx(type_name = "workspace_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRole {
    /// Everything, including deleting the workspace and choosing the other owners. There is always at least one.
    Owner,
    /// Looks after the members and the name: can invite members, remove members, rename.
    Admin,
    /// Works in it.
    Member,
}

impl WorkspaceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceRole::Owner => "owner",
            WorkspaceRole::Admin => "admin",
            WorkspaceRole::Member => "member",
        }
    }
}

use WorkspaceRole::{Admin, Member, Owner};

/// Naming the workspace.
pub fn may_rename(actor: WorkspaceRole) -> bool {
    matches!(actor, Owner | Admin)
}

/// Ending the workspace for everyone.
pub fn may_delete(actor: WorkspaceRole) -> bool {
    actor == Owner
}

/// Seeing the members' email addresses and the invitations that are out. A member sees names.
pub fn may_manage_people(actor: WorkspaceRole) -> bool {
    matches!(actor, Owner | Admin)
}

/// Inviting someone as `invited`. Nobody is invited as an owner (an owner makes an owner of someone who has
/// joined); an admin can bring in members.
pub fn may_invite(actor: WorkspaceRole, invited: WorkspaceRole) -> bool {
    match actor {
        Owner => invited != Owner,
        Admin => invited == Member,
        Member => false,
    }
}

/// Changing what somebody else is. Only an owner decides who is an admin or an owner.
pub fn may_change_role(actor: WorkspaceRole) -> bool {
    actor == Owner
}

/// Removing somebody else (leaving oneself is always allowed, subject to being the last owner).
pub fn may_remove(actor: WorkspaceRole, target: WorkspaceRole) -> bool {
    match actor {
        Owner => true,
        Admin => target == Member,
        Member => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [WorkspaceRole; 3] = [Owner, Admin, Member];

    #[test]
    fn only_owners_and_admins_rename_and_see_people() {
        assert_eq!(ALL.map(may_rename), [true, true, false]);
        assert_eq!(ALL.map(may_manage_people), [true, true, false]);
    }

    #[test]
    fn only_an_owner_deletes_a_workspace_or_changes_roles() {
        assert_eq!(ALL.map(may_delete), [true, false, false]);
        assert_eq!(ALL.map(may_change_role), [true, false, false]);
    }

    #[test]
    fn nobody_is_invited_as_an_owner_and_an_admin_invites_only_members() {
        for invited in ALL {
            assert_eq!(
                may_invite(Owner, invited),
                invited != Owner,
                "owner inviting {invited:?}"
            );
            assert_eq!(
                may_invite(Admin, invited),
                invited == Member,
                "admin inviting {invited:?}"
            );
            assert!(
                !may_invite(Member, invited),
                "a member inviting {invited:?}"
            );
        }
    }

    #[test]
    fn an_admin_removes_members_and_nobody_else() {
        for target in ALL {
            assert!(may_remove(Owner, target));
            assert_eq!(
                may_remove(Admin, target),
                target == Member,
                "admin removing {target:?}"
            );
            assert!(!may_remove(Member, target));
        }
    }

    #[test]
    fn roles_are_written_the_way_the_database_and_the_api_say_them() {
        assert_eq!(ALL.map(WorkspaceRole::as_str), ["owner", "admin", "member"]);
        assert_eq!(serde_json::to_string(&Admin).unwrap(), "\"admin\"");
        assert_eq!(
            serde_json::from_str::<WorkspaceRole>("\"member\"").unwrap(),
            Member
        );
        assert!(serde_json::from_str::<WorkspaceRole>("\"Owner\"").is_err());
        assert!(serde_json::from_str::<WorkspaceRole>("\"root\"").is_err());
    }
}
