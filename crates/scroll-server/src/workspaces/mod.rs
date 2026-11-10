//! The `workspaces` and `memberships` tables, invites to them, and what each role may do.

pub mod invites;

use std::error::Error;

use deadpool_postgres::{Pool, Transaction};
use scroll_api::accounts::Membership;
pub use scroll_core::id::Role;
use scroll_core::id::{UserId, WorkspaceId};
use tokio_postgres::types::{FromSql, Type};

use self::invites::InviteGrant;
use crate::auth::{AccountError, secrets};
use crate::db::DbError;

pub fn can_invite(role: Role) -> bool {
    role == Role::Owner
}

const ROLES: [Role; 3] = [Role::Owner, Role::Editor, Role::Viewer];

/// The name a role is stored under; the `memberships` and `invites` CHECK constraints list these.
pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Owner => "owner",
        Role::Editor => "editor",
        Role::Viewer => "viewer",
    }
}

pub struct StoredRole(pub Role);

impl<'a> FromSql<'a> for StoredRole {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let name = <&str>::from_sql(ty, raw)?;
        let role = ROLES
            .into_iter()
            .find(|role| role_name(*role) == name)
            .ok_or_else(|| format!("unknown role {name:?}"))?;
        Ok(StoredRole(role))
    }

    fn accepts(ty: &Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

pub struct NewWorkspace {
    pub id: WorkspaceId,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkspaceGrant {
    pub workspace_id: WorkspaceId,
    pub role: Role,
}

pub async fn workspace_role(
    pool: &Pool,
    user_id: UserId,
    workspace_id: WorkspaceId,
) -> Result<Option<Role>, DbError> {
    let client = pool.get().await?;
    let row = client
        .query_opt(
            "SELECT role FROM memberships WHERE workspace_id = $1 AND user_id = $2",
            &[&workspace_id.0, &user_id.0],
        )
        .await?;
    Ok(row.map(|row| row.get::<_, StoredRole>("role").0))
}

pub async fn list_workspaces(pool: &Pool, user_id: UserId) -> Result<Vec<Membership>, DbError> {
    let client = pool.get().await?;
    let rows = client
        .query(
            "SELECT w.id, w.name, m.role
             FROM memberships m JOIN workspaces w ON w.id = m.workspace_id
             WHERE m.user_id = $1
             ORDER BY w.name, w.id",
            &[&user_id.0],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| Membership {
            workspace_id: WorkspaceId(row.get("id")),
            name: row.get("name"),
            role: row.get::<_, StoredRole>("role").0,
        })
        .collect())
}

/// The creator becomes the workspace's owner.
pub async fn create_workspace(
    pool: &Pool,
    creator: UserId,
    workspace: NewWorkspace,
) -> Result<Membership, AccountError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let inserted = tx
        .execute(
            "INSERT INTO workspaces (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
            &[&workspace.id.0, &workspace.name],
        )
        .await?;
    if inserted == 0 {
        return Err(AccountError::WorkspaceExists);
    }
    let grant = WorkspaceGrant {
        workspace_id: workspace.id,
        role: Role::Owner,
    };
    add_membership(&tx, creator, grant).await?;
    tx.commit().await?;
    Ok(Membership {
        workspace_id: workspace.id,
        name: workspace.name,
        role: Role::Owner,
    })
}

/// For a signed-in user joining another workspace. An existing membership keeps its role.
pub async fn accept_invite(
    pool: &Pool,
    user_id: UserId,
    invite_code: &str,
) -> Result<Membership, AccountError> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let invite_hash = secrets::hash_secret(invite_code);
    let Some(InviteGrant::Workspace(grant)) = invites::claim(&tx, &invite_hash, user_id).await?
    else {
        return Err(AccountError::InvalidInvite);
    };
    add_membership(&tx, user_id, grant).await?;
    let row = tx
        .query_one(
            "SELECT w.name, m.role
             FROM memberships m JOIN workspaces w ON w.id = m.workspace_id
             WHERE m.workspace_id = $1 AND m.user_id = $2",
            &[&grant.workspace_id.0, &user_id.0],
        )
        .await?;
    tx.commit().await?;
    Ok(Membership {
        workspace_id: grant.workspace_id,
        name: row.get("name"),
        role: row.get::<_, StoredRole>("role").0,
    })
}

pub async fn add_membership(
    tx: &Transaction<'_>,
    user_id: UserId,
    grant: WorkspaceGrant,
) -> Result<(), tokio_postgres::Error> {
    tx.execute(
        "INSERT INTO memberships (workspace_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (workspace_id, user_id) DO NOTHING",
        &[&grant.workspace_id.0, &user_id.0, &role_name(grant.role)],
    )
    .await?;
    Ok(())
}
