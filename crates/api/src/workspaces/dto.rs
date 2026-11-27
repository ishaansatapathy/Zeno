use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::secret::Secret;
use crate::workspaces::permissions::WorkspaceRole;
use crate::workspaces::repo::{InviteRow, MemberRow, SummaryRow};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspaceRequest {
    /// 2 to 80 characters, trimmed. Not unique: two teams may be called the same.
    #[schema(min_length = 2, max_length = 80)]
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkspaceRequest {
    #[schema(min_length = 2, max_length = 80)]
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRoleRequest {
    pub role: WorkspaceRole,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInviteRequest {
    /// Where the invitation is sent. Only the account that has this address, verified, can accept it.
    #[schema(format = Email)]
    pub email: String,
    /// What they will be. `admin` or `member`: nobody is invited as an owner.
    pub role: WorkspaceRole,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptInviteRequest {
    /// The token in the emailed link.
    #[schema(value_type = String)]
    pub token: Secret,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
    /// What the person asking is in it.
    pub role: WorkspaceRole,
    pub member_count: i64,
    pub created_at: DateTime<Utc>,
}

impl From<SummaryRow> for WorkspaceResponse {
    fn from(row: SummaryRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            role: row.role,
            member_count: row.member_count,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MemberResponse {
    pub user_id: Uuid,
    pub name: String,
    /// Only owners and admins see the addresses of the others.
    pub email: Option<String>,
    pub role: WorkspaceRole,
    pub joined_at: DateTime<Utc>,
    /// Is this the person asking.
    pub you: bool,
}

impl MemberResponse {
    pub fn build(row: MemberRow, viewer: Uuid, sees_emails: bool) -> Self {
        let you = row.user_id == viewer;
        Self {
            user_id: row.user_id,
            name: row.name,
            email: if sees_emails || you { row.email } else { None },
            role: row.role,
            joined_at: row.joined_at,
            you,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteResponse {
    pub id: Uuid,
    pub email: String,
    pub role: WorkspaceRole,
    /// Who asked, by name. Empty if they have since left Zeno.
    pub invited_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl From<InviteRow> for InviteResponse {
    fn from(row: InviteRow) -> Self {
        Self {
            id: row.id,
            email: row.email,
            role: row.role,
            invited_by: row.invited_by_name,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceEnvelope {
    pub workspace: WorkspaceResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceListEnvelope {
    /// Newest first.
    pub workspaces: Vec<WorkspaceResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkspaceDetailEnvelope {
    pub workspace: WorkspaceResponse,
    /// In the order they joined.
    pub members: Vec<MemberResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteEnvelope {
    pub invite: InviteResponse,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InviteListEnvelope {
    /// The invitations still waiting for an answer, newest first.
    pub invites: Vec<InviteResponse>,
}
