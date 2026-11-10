//! Workspaces and the invites into them.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use scroll_api::accounts::{
    AcceptInviteRequest, CreateInviteRequest, CreateWorkspaceRequest, InviteCreated, Membership,
};
use scroll_core::id::WorkspaceId;

use super::validation::{check_invite_code, check_workspace_name};
use super::{ApiError, SignedIn};
use crate::state::AppState;
use crate::workspaces::invites::{self, NewInvite};
use crate::workspaces::{self, NewWorkspace, WorkspaceGrant};

pub async fn list_workspaces(
    State(state): State<AppState>,
    signed_in: SignedIn,
) -> Result<Json<Vec<Membership>>, ApiError> {
    Ok(Json(
        workspaces::list_workspaces(&state.pool, signed_in.user_id).await?,
    ))
}

pub async fn create_workspace(
    State(state): State<AppState>,
    signed_in: SignedIn,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<Membership>), ApiError> {
    let workspace = NewWorkspace {
        id: request.id,
        name: check_workspace_name(&request.name)?,
    };
    let membership =
        workspaces::create_workspace(&state.pool, signed_in.user_id, workspace).await?;
    Ok((StatusCode::CREATED, Json(membership)))
}

pub async fn create_invite(
    State(state): State<AppState>,
    signed_in: SignedIn,
    Path(workspace_id): Path<WorkspaceId>,
    Json(request): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<InviteCreated>), ApiError> {
    let role = workspaces::workspace_role(&state.pool, signed_in.user_id, workspace_id).await?;
    if !role.is_some_and(workspaces::can_invite) {
        return Err(ApiError::Forbidden);
    }
    let invite = NewInvite {
        grant: Some(WorkspaceGrant {
            workspace_id,
            role: request.role,
        }),
        created_by: Some(signed_in.user_id),
    };
    let invite_code = invites::create_invite(&state.pool, &invite).await?;
    Ok((StatusCode::CREATED, Json(InviteCreated { invite_code })))
}

pub async fn accept_invite(
    State(state): State<AppState>,
    signed_in: SignedIn,
    Json(request): Json<AcceptInviteRequest>,
) -> Result<Json<Membership>, ApiError> {
    let invite_code = check_invite_code(request.invite_code)?;
    Ok(Json(
        workspaces::accept_invite(&state.pool, signed_in.user_id, &invite_code).await?,
    ))
}
