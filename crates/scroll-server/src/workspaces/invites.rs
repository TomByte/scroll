//! The `invites` table. An invite without a workspace grants sign-up only; that is how the first user gets in.

use deadpool_postgres::{Pool, Transaction};
use scroll_core::id::{Role, UserId, WorkspaceId};
use uuid::Uuid;

use super::{StoredRole, WorkspaceGrant};
use crate::auth::{AccountError, secrets};
use crate::db::DbError;

const INVITE_LIFETIME_DAYS: i32 = 7;

pub struct NewInvite {
    /// `None` creates a sign-up-only invite.
    pub grant: Option<WorkspaceGrant>,
    pub created_by: Option<UserId>,
}

/// What a claimed invite gives beyond the right to have an account.
pub enum InviteGrant {
    SignUpOnly,
    Workspace(WorkspaceGrant),
}

/// Returns the invite code. Only its hash is stored, so this is the one chance to read it.
pub async fn create_invite(pool: &Pool, invite: &NewInvite) -> Result<String, DbError> {
    let code = secrets::generate_secret();
    let client = pool.get().await?;
    client
        .execute(
            "INSERT INTO invites (code_hash, workspace_id, role, created_by, expires_at)
             VALUES ($1, $2, $3, $4, now() + make_interval(days => $5))",
            &[
                &secrets::hash_secret(&code),
                &invite.grant.map(|grant| grant.workspace_id.0),
                &invite.grant.map(|grant| super::role_name(grant.role)),
                &invite.created_by.map(|user| user.0),
                &INVITE_LIFETIME_DAYS,
            ],
        )
        .await?;
    Ok(code)
}

/// Marks the invite used in the same statement that checks it, so two people cannot both claim it.
pub async fn claim(
    tx: &Transaction<'_>,
    invite_hash: &[u8],
    user_id: UserId,
) -> Result<Option<InviteGrant>, tokio_postgres::Error> {
    let row = tx
        .query_opt(
            "UPDATE invites SET accepted_by = $2, accepted_at = now()
             WHERE code_hash = $1 AND accepted_at IS NULL AND expires_at > now()
             RETURNING workspace_id, role",
            &[&invite_hash, &user_id.0],
        )
        .await?;
    Ok(row.map(|row| {
        let workspace_id: Option<Uuid> = row.get("workspace_id");
        let role: Option<Role> = row
            .get::<_, Option<StoredRole>>("role")
            .map(|stored| stored.0);
        match workspace_id.zip(role) {
            Some((workspace_id, role)) => InviteGrant::Workspace(WorkspaceGrant {
                workspace_id: WorkspaceId(workspace_id),
                role,
            }),
            None => InviteGrant::SignUpOnly,
        }
    }))
}

/// Claims the invite for a new account and grants what it carries.
pub async fn apply(
    tx: &Transaction<'_>,
    invite_hash: &[u8],
    user_id: UserId,
) -> Result<(), AccountError> {
    match claim(tx, invite_hash, user_id).await? {
        None => Err(AccountError::InvalidInvite),
        Some(InviteGrant::SignUpOnly) => Ok(()),
        Some(InviteGrant::Workspace(grant)) => Ok(super::add_membership(tx, user_id, grant).await?),
    }
}
