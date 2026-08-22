//! Postgres access for auth-service. Literal SQL only (sqlx 0.9), runtime
//! checked. Every hash column holds SHA-256 bytes; no clear-text token or
//! secret is ever written.

use afixo_common::ids::new_id;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// oauth states

pub async fn insert_state(
    pool: &PgPool,
    state: &str,
    redirect_to: &str,
    expires_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query("insert into oauth_states (state, redirect_to, expires_at) values ($1, $2, $3)")
        .bind(state)
        .bind(redirect_to)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Single use: the row is deleted in the same statement that reads it.
pub async fn consume_state(pool: &PgPool, state: &str) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar(
        "delete from oauth_states where state = $1 and expires_at > now() returning redirect_to",
    )
    .bind(state)
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------------
// github identities

#[derive(Debug, FromRow)]
pub struct IdentityRow {
    pub subject_id: Uuid,
    pub login: String,
}

pub async fn find_identity(pool: &PgPool, github_id: i64) -> sqlx::Result<Option<IdentityRow>> {
    sqlx::query_as("select subject_id, login from github_identities where github_id = $1")
        .bind(github_id)
        .fetch_optional(pool)
        .await
}

pub async fn link_identity(
    pool: &PgPool,
    github_id: i64,
    subject_id: Uuid,
    login: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into github_identities (github_id, subject_id, login, last_login_at) \
         values ($1, $2, $3, now())",
    )
    .bind(github_id)
    .bind(subject_id)
    .bind(login)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch_identity(pool: &PgPool, github_id: i64, login: &str) -> sqlx::Result<()> {
    sqlx::query(
        "update github_identities set login = $2, last_login_at = now() where github_id = $1",
    )
    .bind(github_id)
    .bind(login)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// sessions

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub family_id: Uuid,
    pub access_expires_at: OffsetDateTime,
    pub refresh_expires_at: OffsetDateTime,
    pub rotated_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

/// A session about to be written: both hashes plus expiries.
#[derive(Debug)]
pub struct NewSession {
    pub subject_id: Uuid,
    pub family_id: Uuid,
    pub access_hash: [u8; 32],
    pub refresh_hash: [u8; 32],
    pub access_expires_at: OffsetDateTime,
    pub refresh_expires_at: OffsetDateTime,
}

pub async fn insert_session(pool: &PgPool, s: &NewSession) -> sqlx::Result<SessionRow> {
    sqlx::query_as(
        "insert into sessions \
            (id, subject_id, family_id, access_hash, refresh_hash, access_expires_at, refresh_expires_at) \
         values ($1, $2, $3, $4, $5, $6, $7) \
         returning id, subject_id, family_id, access_expires_at, refresh_expires_at, rotated_at, revoked_at",
    )
    .bind(new_id())
    .bind(s.subject_id)
    .bind(s.family_id)
    .bind(s.access_hash.as_slice())
    .bind(s.refresh_hash.as_slice())
    .bind(s.access_expires_at)
    .bind(s.refresh_expires_at)
    .fetch_one(pool)
    .await
}

pub async fn session_by_refresh_hash(
    pool: &PgPool,
    refresh_hash: &[u8],
) -> sqlx::Result<Option<SessionRow>> {
    sqlx::query_as(
        "select id, subject_id, family_id, access_expires_at, refresh_expires_at, rotated_at, revoked_at \
           from sessions where refresh_hash = $1",
    )
    .bind(refresh_hash)
    .fetch_optional(pool)
    .await
}

/// The live session for an access token: not revoked, not expired.
pub async fn active_session_by_access_hash(
    pool: &PgPool,
    access_hash: &[u8],
) -> sqlx::Result<Option<SessionRow>> {
    sqlx::query_as(
        "select id, subject_id, family_id, access_expires_at, refresh_expires_at, rotated_at, revoked_at \
           from sessions \
          where access_hash = $1 and revoked_at is null and access_expires_at > now()",
    )
    .bind(access_hash)
    .fetch_optional(pool)
    .await
}

/// Mark `old_id` as rotated and write its successor in one transaction.
pub async fn rotate_session(
    pool: &PgPool,
    old_id: Uuid,
    next: &NewSession,
) -> sqlx::Result<SessionRow> {
    let mut tx = pool.begin().await?;
    sqlx::query("update sessions set rotated_at = now() where id = $1 and rotated_at is null")
        .bind(old_id)
        .execute(&mut *tx)
        .await?;
    let row: SessionRow = sqlx::query_as(
        "insert into sessions \
            (id, subject_id, family_id, access_hash, refresh_hash, access_expires_at, refresh_expires_at) \
         values ($1, $2, $3, $4, $5, $6, $7) \
         returning id, subject_id, family_id, access_expires_at, refresh_expires_at, rotated_at, revoked_at",
    )
    .bind(new_id())
    .bind(next.subject_id)
    .bind(next.family_id)
    .bind(next.access_hash.as_slice())
    .bind(next.refresh_hash.as_slice())
    .bind(next.access_expires_at)
    .bind(next.refresh_expires_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Reuse detection fired: every session in the family dies.
pub async fn revoke_family(pool: &PgPool, family_id: Uuid) -> sqlx::Result<u64> {
    let result = sqlx::query(
        "update sessions set revoked_at = now() where family_id = $1 and revoked_at is null",
    )
    .bind(family_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn revoke_by_access_hash(pool: &PgPool, access_hash: &[u8]) -> sqlx::Result<u64> {
    let result = sqlx::query(
        "update sessions set revoked_at = now() where access_hash = $1 and revoked_at is null",
    )
    .bind(access_hash)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// requesters

#[derive(Debug, Clone, FromRow)]
pub struct RequesterRow {
    pub id: Uuid,
    pub owner_subject_id: Uuid,
    pub client_id: String,
    pub name: String,
    pub secret_hash: Vec<u8>,
    pub created_at: OffsetDateTime,
    pub rotated_at: Option<OffsetDateTime>,
}

pub async fn insert_requester(
    pool: &PgPool,
    owner_subject_id: Uuid,
    client_id: &str,
    name: &str,
    secret_hash: &[u8],
) -> sqlx::Result<RequesterRow> {
    sqlx::query_as(
        "insert into requesters (id, owner_subject_id, client_id, name, secret_hash) \
         values ($1, $2, $3, $4, $5) \
         returning id, owner_subject_id, client_id, name, secret_hash, created_at, rotated_at",
    )
    .bind(new_id())
    .bind(owner_subject_id)
    .bind(client_id)
    .bind(name)
    .bind(secret_hash)
    .fetch_one(pool)
    .await
}

pub async fn requester_by_client_id(
    pool: &PgPool,
    client_id: &str,
) -> sqlx::Result<Option<RequesterRow>> {
    sqlx::query_as(
        "select id, owner_subject_id, client_id, name, secret_hash, created_at, rotated_at \
           from requesters where client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
}

pub async fn requester_by_id(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<RequesterRow>> {
    sqlx::query_as(
        "select id, owner_subject_id, client_id, name, secret_hash, created_at, rotated_at \
           from requesters where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// `owner = None` is the public directory; `Some(id)` only that subject's clients.
pub async fn list_requesters(
    pool: &PgPool,
    owner: Option<Uuid>,
) -> sqlx::Result<Vec<RequesterRow>> {
    sqlx::query_as(
        "select id, owner_subject_id, client_id, name, secret_hash, created_at, rotated_at \
           from requesters \
          where ($1::uuid is null or owner_subject_id = $1) \
          order by created_at desc",
    )
    .bind(owner)
    .fetch_all(pool)
    .await
}

/// Ownership is part of the predicate: a foreign requester id rotates nothing.
/// Existing tokens die with the old secret.
pub async fn rotate_secret(
    pool: &PgPool,
    requester_id: Uuid,
    owner_subject_id: Uuid,
    secret_hash: &[u8],
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "update requesters set secret_hash = $3, rotated_at = now() \
          where id = $1 and owner_subject_id = $2",
    )
    .bind(requester_id)
    .bind(owner_subject_id)
    .bind(secret_hash)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 1 {
        sqlx::query("delete from requester_tokens where requester_id = $1")
            .bind(requester_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(updated == 1)
}

// ---------------------------------------------------------------------------
// requester tokens

pub async fn insert_token(
    pool: &PgPool,
    token_hash: &[u8],
    requester_id: Uuid,
    expires_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query(
        "insert into requester_tokens (token_hash, requester_id, expires_at) values ($1, $2, $3)",
    )
    .bind(token_hash)
    .bind(requester_id)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The requester behind a live token, with its expiry.
pub async fn live_token(
    pool: &PgPool,
    token_hash: &[u8],
) -> sqlx::Result<Option<(Uuid, OffsetDateTime)>> {
    sqlx::query_as(
        "select requester_id, expires_at from requester_tokens \
          where token_hash = $1 and expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------------
// hygiene

/// Delete rows that can never match again. Returns (states, tokens, sessions).
pub async fn delete_expired(pool: &PgPool) -> sqlx::Result<(u64, u64, u64)> {
    let states = sqlx::query("delete from oauth_states where expires_at <= now()")
        .execute(pool)
        .await?
        .rows_affected();
    let tokens = sqlx::query("delete from requester_tokens where expires_at <= now()")
        .execute(pool)
        .await?
        .rows_affected();
    let sessions =
        sqlx::query("delete from sessions where refresh_expires_at <= now() - interval '1 day'")
            .execute(pool)
            .await?
            .rows_affected();
    Ok((states, tokens, sessions))
}
