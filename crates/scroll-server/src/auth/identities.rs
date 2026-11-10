//! The `identities` table: people a sign-in provider vouches for, by the provider's own stable id.

use deadpool_postgres::{Pool, Transaction};
use scroll_api::accounts::SignUpMode;
use scroll_core::id::UserId;

use super::{AccountError, sessions, users};
use crate::workspaces::invites;

/// A person a sign-in provider vouches for, with an email it has verified.
pub struct VerifiedIdentity {
    pub provider: &'static str,
    pub subject: String,
    pub email: String,
    /// The person's name at the provider, if it shares one.
    pub name: Option<String>,
}

/// Who may sign in with a verified identity, and how a new account gets in.
pub struct IdentitySignIn<'a> {
    pub identity: &'a VerifiedIdentity,
    pub sign_up: SignUpMode,
    /// The hash of an invite brought along, used only when an account is created.
    pub invite_hash: Option<Vec<u8>>,
}

/// The account a verified identity signed in to, and the sessions ended by taking the account over, by token
/// hash.
pub struct IdentityAccount {
    pub user_id: UserId,
    pub ended_sessions: Vec<Vec<u8>>,
}

/// Signs in the account with this identity. Without one, the account with the identity's email gets the
/// identity; without that either, a new account is created if the sign-up mode or the invite allows.
///
/// Taking over an existing account by email: nothing ever proved that whoever chose its password owns the
/// email, but the provider just proved this person does. So the password is cleared and every session of the
/// account ends: someone who registered the address first, to wait for its owner, is locked out rather than
/// handed a shared account. The owner signs in with the provider from then on.
pub async fn sign_in_with_identity(
    pool: &Pool,
    sign_in: IdentitySignIn<'_>,
) -> Result<IdentityAccount, AccountError> {
    let identity = sign_in.identity;
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    let account = match linked_user(&tx, identity).await? {
        Some(user_id) => IdentityAccount {
            user_id,
            ended_sessions: Vec::new(),
        },
        None => link(&tx, &sign_in).await?,
    };
    if let Some(name) = &identity.name {
        users::set_display_name(&tx, account.user_id, name).await?;
    }
    tx.commit().await?;
    Ok(account)
}

async fn linked_user(
    tx: &Transaction<'_>,
    identity: &VerifiedIdentity,
) -> Result<Option<UserId>, tokio_postgres::Error> {
    let row = tx
        .query_opt(
            "SELECT user_id FROM identities WHERE provider = $1 AND subject = $2",
            &[&identity.provider, &identity.subject],
        )
        .await?;
    Ok(row.map(|row| UserId(row.get("user_id"))))
}

/// Gives the identity to the account with its email, or to a new account.
async fn link(
    tx: &Transaction<'_>,
    sign_in: &IdentitySignIn<'_>,
) -> Result<IdentityAccount, AccountError> {
    let identity = sign_in.identity;
    let account = match users::lock_by_email(tx, &identity.email).await? {
        Some(user_id) => take_over(tx, user_id).await?,
        None => IdentityAccount {
            user_id: create_account(tx, sign_in).await?,
            ended_sessions: Vec::new(),
        },
    };
    let linked = tx
        .execute(
            "INSERT INTO identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            &[
                &identity.provider,
                &identity.subject,
                &account.user_id.0,
                &identity.email,
            ],
        )
        .await?;
    // The account already has another identity at this provider (say, an earlier Google account with this
    // address), or a concurrent sign-in linked this one first.
    if linked == 0 {
        return Err(AccountError::IdentityConflict);
    }
    Ok(account)
}

async fn take_over(tx: &Transaction<'_>, user_id: UserId) -> Result<IdentityAccount, AccountError> {
    users::clear_password(tx, user_id).await?;
    Ok(IdentityAccount {
        user_id,
        ended_sessions: sessions::end_all_sessions(tx, user_id).await?,
    })
}

async fn create_account(
    tx: &Transaction<'_>,
    sign_in: &IdentitySignIn<'_>,
) -> Result<UserId, AccountError> {
    if sign_in.sign_up == SignUpMode::Invite && sign_in.invite_hash.is_none() {
        return Err(AccountError::InviteRequired);
    }
    let user_id = users::create_without_password(tx, &sign_in.identity.email).await?;
    if let Some(invite_hash) = &sign_in.invite_hash {
        invites::apply(tx, invite_hash, user_id).await?;
    }
    Ok(user_id)
}
