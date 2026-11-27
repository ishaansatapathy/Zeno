use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::extractor::{ApiJson, AuthUser};
use crate::cursor::TimeCursor;
use crate::error::{ApiError, ErrorBody};
use crate::ratelimit::{self, policies};
use crate::state::AppState;
use crate::workspaces::dto::{
    AcceptInviteRequest, ChangeRoleRequest, CreateInviteRequest, CreateWorkspaceRequest,
    InviteEnvelope, InviteListEnvelope, InviteResponse, TeamRecordingListEnvelope,
    TeamRecordingsQuery, UpdateWorkspaceRequest, WorkspaceDetailEnvelope, WorkspaceEnvelope,
    WorkspaceListEnvelope, WorkspaceResponse,
};
use crate::workspaces::service;

/// Mounted at `/v1/workspaces`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/:id", get(show).patch(update).delete(remove))
        .route("/:id/members/:user", patch(set_role).delete(drop_member))
        .route("/:id/invites", get(list_invites).post(invite))
        .route("/:id/invites/:invite", delete(revoke_invite))
        .route("/:id/recordings", get(team_recordings))
        .route("/:id/recordings/:recording", put(add_rec).delete(take_out))
}

/// Mounted at `/v1/invites`: the one thing a person does with an invitation they were sent.
pub fn invites_router() -> Router<AppState> {
    Router::new().route("/accept", post(accept))
}

/// An id that is not a UUID names nothing: it is "not found", like any other workspace that is not there.
fn parse(raw: &str, what: &'static str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::Http {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: format!("{what} not found."),
    })
}

async fn reading(state: &AppState, user: &AuthUser) -> Result<(), ApiError> {
    ratelimit::enforce_user(state, &policies::WORKSPACE_READ, user.id).await
}

async fn writing(state: &AppState, user: &AuthUser) -> Result<(), ApiError> {
    ratelimit::enforce_user(state, &policies::WORKSPACE_WRITE, user.id).await
}

fn one(workspace: WorkspaceResponse) -> Json<WorkspaceEnvelope> {
    Json(WorkspaceEnvelope { workspace })
}

/// Starts a workspace, with the caller as its owner.
///
/// An account may own five, and start ten an hour.
#[utoipa::path(
    post,
    path = "/v1/workspaces",
    operation_id = "workspaces_create",
    tag = "workspaces",
    security(("session" = [])),
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Made.", body = WorkspaceEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 409, description = "`too_many_workspaces`: the account already owns five.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceEnvelope>), ApiError> {
    writing(&state, &user).await?;
    Ok((
        StatusCode::CREATED,
        one(service::create(&state, &user, &body.name).await?),
    ))
}

/// The workspaces the caller is in, newest first, each with what they are in it.
#[utoipa::path(
    get,
    path = "/v1/workspaces",
    operation_id = "workspaces_list",
    tag = "workspaces",
    security(("session" = [])),
    responses(
        (status = 200, description = "All of them: nobody is in enough for a page to be needed.", body = WorkspaceListEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<WorkspaceListEnvelope>, ApiError> {
    reading(&state, &user).await?;
    Ok(Json(WorkspaceListEnvelope {
        workspaces: service::list(&state, &user).await?,
    }))
}

/// One workspace and the people in it.
///
/// Owners and admins see every member's email address; a member sees names (and their own address). Somebody who
/// is not in the workspace gets `404`, as for one that does not exist.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{id}",
    operation_id = "workspaces_show",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace.")),
    responses(
        (status = 200, description = "The workspace and its members.", body = WorkspaceDetailEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceDetailEnvelope>, ApiError> {
    reading(&state, &user).await?;
    let (workspace, members) = service::detail(&state, &user, parse(&id, "Workspace")?).await?;
    Ok(Json(WorkspaceDetailEnvelope { workspace, members }))
}

/// Renames the workspace. Owners and admins.
#[utoipa::path(
    patch,
    path = "/v1/workspaces/{id}",
    operation_id = "workspaces_update",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace.")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Renamed.", body = WorkspaceEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: a member may not, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceEnvelope>, ApiError> {
    writing(&state, &user).await?;
    Ok(one(service::rename(
        &state,
        &user,
        parse(&id, "Workspace")?,
        &body.name,
    )
    .await?))
}

/// Ends the workspace for everyone in it. Owners only.
///
/// Its members lose access at once and its open invitations stop working. Nothing a member made on their own is
/// touched: it was never the workspace's.
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{id}",
    operation_id = "workspaces_delete",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace.")),
    responses(
        (status = 204, description = "Ended."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: only an owner may, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    service::delete(&state, &user, parse(&id, "Workspace")?).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Changes what somebody is in the workspace. Owners only.
///
/// The last owner cannot be made anything else: a workspace always has one.
#[utoipa::path(
    patch,
    path = "/v1/workspaces/{id}/members/{user}",
    operation_id = "workspaces_set_role",
    tag = "workspaces",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "The workspace."),
        ("user" = Uuid, Path, description = "The member."),
    ),
    request_body = ChangeRoleRequest,
    responses(
        (status = 204, description = "Changed (or already so)."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: only an owner may, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`last_owner`: that would leave the workspace with none.", body = ErrorBody),
        (status = 422, description = "`invalid_request`: not one of `owner`, `admin`, `member`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn set_role(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, target)): Path<(String, String)>,
    ApiJson(body): ApiJson<ChangeRoleRequest>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    let (id, target) = (parse(&id, "Workspace")?, parse(&target, "Member")?);
    service::change_role(&state, &user, id, target, body.role).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a member, or leaves (when `user` is the caller).
///
/// Owners remove anyone; admins remove members. Anyone may leave, except the last owner.
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{id}/members/{user}",
    operation_id = "workspaces_drop_member",
    tag = "workspaces",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "The workspace."),
        ("user" = Uuid, Path, description = "The member."),
    ),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: this role may not remove that one, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`last_owner`: the only owner cannot leave.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn drop_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, target)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    let (id, target) = (parse(&id, "Workspace")?, parse(&target, "Member")?);
    service::remove_member(&state, &user, id, target).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Invites somebody by email address.
///
/// The email carries a link that works for seven days and only for the account that has that address, verified.
/// Owners invite admins and members; admins invite members. Twenty an hour, and a workspace holds fifty people
/// counting the invitations still open. The answer never says whether the address has an account.
#[utoipa::path(
    post,
    path = "/v1/workspaces/{id}/invites",
    operation_id = "workspaces_invite",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace.")),
    request_body = CreateInviteRequest,
    responses(
        (status = 201, description = "Sent.", body = InviteEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: this role may not invite that one, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`already_member`, `already_invited` (revoke the open one to send another) or `workspace_full`.", body = ErrorBody),
        (status = 422, description = "`validation_failed` (see `fields`) or `invalid_request`.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CreateInviteRequest>,
) -> Result<(StatusCode, Json<InviteEnvelope>), ApiError> {
    writing(&state, &user).await?;
    let invite: InviteResponse = service::invite(
        &state,
        &user,
        parse(&id, "Workspace")?,
        &body.email,
        body.role,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(InviteEnvelope { invite })))
}

/// The invitations still waiting for an answer. Owners and admins.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{id}/invites",
    operation_id = "workspaces_invites",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace.")),
    responses(
        (status = 200, description = "Newest first. Never carries the token.", body = InviteListEnvelope),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: a member may not see them.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn list_invites(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<InviteListEnvelope>, ApiError> {
    reading(&state, &user).await?;
    Ok(Json(InviteListEnvelope {
        invites: service::list_invites(&state, &user, parse(&id, "Workspace")?).await?,
    }))
}

/// Takes an invitation back. Owners and admins. The link stops working at once.
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{id}/invites/{invite}",
    operation_id = "workspaces_revoke_invite",
    tag = "workspaces",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "The workspace."),
        ("invite" = Uuid, Path, description = "The invitation."),
    ),
    responses(
        (status = 204, description = "Revoked."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: a member may not, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`: no such workspace, or no such open invitation in it.", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn revoke_invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, invite)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    let (id, invite) = (parse(&id, "Workspace")?, parse(&invite, "Invitation")?);
    service::revoke_invite(&state, &user, id, invite).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Accepts an invitation, with the token from the emailed link.
///
/// The caller must be signed in as the account that has the address the invitation was sent to, verified. A link
/// that is wrong, used, revoked or out of time is one answer (`invalid_token`): nothing says which.
#[utoipa::path(
    post,
    path = "/v1/invites/accept",
    operation_id = "invites_accept",
    tag = "workspaces",
    security(("session" = [])),
    request_body = AcceptInviteRequest,
    responses(
        (status = 200, description = "Joined (or already in it). The workspace, with the role the caller now has.", body = WorkspaceEnvelope),
        (status = 400, description = "`invalid_token`: wrong, used, revoked, expired, or its workspace is gone.", body = ErrorBody),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`wrong_account`: the invitation was sent to another address, or this one is not verified; or `forbidden_origin`.", body = ErrorBody),
        (status = 409, description = "`workspace_full`", body = ErrorBody),
        (status = 422, description = "`invalid_request`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<AcceptInviteRequest>,
) -> Result<Json<WorkspaceEnvelope>, ApiError> {
    writing(&state, &user).await?;
    Ok(one(
        service::accept(&state, &user, body.token.expose()).await?
    ))
}

/// The recordings in the workspace, newest first, a page at a time.
///
/// Anyone in it can see this list and watch what is in it. Somebody who is not in the workspace gets `404`.
#[utoipa::path(
    get,
    path = "/v1/workspaces/{id}/recordings",
    operation_id = "workspaces_recordings",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace."), TeamRecordingsQuery),
    responses(
        (status = 200, description = "One page.", body = TeamRecordingListEnvelope),
        (status = 400, description = "`invalid_cursor`", body = ErrorBody),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn team_recordings(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<TeamRecordingsQuery>,
) -> Result<Json<TeamRecordingListEnvelope>, ApiError> {
    reading(&state, &user).await?;
    let cursor = query
        .cursor
        .as_deref()
        .map(TimeCursor::decode)
        .transpose()?;
    let (recordings, next) =
        service::recordings(&state, &user, parse(&id, "Workspace")?, cursor, query.limit).await?;
    Ok(Json(TeamRecordingListEnvelope {
        recordings,
        next_cursor: next.map(TimeCursor::encode),
    }))
}

/// Puts one of your recordings in the workspace, so everyone in it can see and watch it.
///
/// It stays yours: you edit it, share it and delete it. A recording is in one workspace at a time, so putting it
/// in another moves it. Repeating the call changes nothing. A recording that is not yours, or is deleted, is
/// `404`, as is a workspace you are not in.
#[utoipa::path(
    put,
    path = "/v1/workspaces/{id}/recordings/{recording}",
    operation_id = "workspaces_add_recording",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace."), ("recording" = Uuid, Path, description = "One of your recordings.")),
    responses(
        (status = 204, description = "It is in the workspace."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden_origin`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn add_rec(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, rec)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    service::add_recording(
        &state,
        &user,
        parse(&id, "Workspace")?,
        parse(&rec, "Recording")?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Takes a recording out of the workspace. It stays in its maker's library.
///
/// Its maker may, and so may the workspace's owners and admins; a member may not take out somebody else's.
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{id}/recordings/{recording}",
    operation_id = "workspaces_take_out_recording",
    tag = "workspaces",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The workspace."), ("recording" = Uuid, Path, description = "A recording in it.")),
    responses(
        (status = 204, description = "It is out. Viewings of it by the workspace's people end at their next request."),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`forbidden`: a member may not take out somebody else's, or `forbidden_origin`.", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 429, description = "`too_many_requests`", body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds until the budget is available again."))),
    )
)]
pub(crate) async fn take_out(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, rec)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    writing(&state, &user).await?;
    service::take_out_recording(
        &state,
        &user,
        parse(&id, "Workspace")?,
        parse(&rec, "Recording")?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
