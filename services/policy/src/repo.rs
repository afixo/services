//! Postgres access for policy-service. Runtime-checked queries (`query_as`)
//! so the workspace builds without a database; switch to `query!` macros with
//! `cargo sqlx prepare` if compile-time checking becomes worth the friction.
//!
//! sqlx 0.9 only accepts `&'static str` SQL (no `format!`), which is why the
//! column list is spelled out in every query instead of being shared.

use afixo_common::time::to_proto;
use afixo_proto::{AllowList, Purpose, Rule};
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct PurposeRow {
    pub name: String,
    pub description: String,
}

impl From<PurposeRow> for Purpose {
    fn from(r: PurposeRow) -> Self {
        Self {
            name: r.name,
            description: r.description,
        }
    }
}

#[derive(Debug, FromRow)]
pub struct RuleRow {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub requester_id: Option<Uuid>,
    pub purpose: Option<String>,
    pub persona_id: Uuid,
    pub max_sensitivity: i16,
    pub allow_keys: Option<Vec<String>>,
    pub priority: i32,
    pub created_seq: i64,
    pub created_at: OffsetDateTime,
}

impl From<RuleRow> for Rule {
    fn from(r: RuleRow) -> Self {
        Self {
            id: r.id.to_string(),
            subject_id: r.subject_id.to_string(),
            requester_id: r.requester_id.map(|u| u.to_string()),
            purpose: r.purpose,
            persona_id: r.persona_id.to_string(),
            max_sensitivity: i32::from(r.max_sensitivity),
            allow_list: r.allow_keys.map(|keys| AllowList { keys }),
            priority: r.priority,
            created_at: Some(to_proto(r.created_at)),
        }
    }
}

pub async fn list_purposes(pool: &PgPool) -> sqlx::Result<Vec<PurposeRow>> {
    sqlx::query_as("select name, description from purposes order by sort_order, name")
        .fetch_all(pool)
        .await
}

pub async fn purpose_exists(pool: &PgPool, name: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar("select exists(select 1 from purposes where name = $1)")
        .bind(name)
        .fetch_one(pool)
        .await
}

pub async fn list_rules(pool: &PgPool, subject_id: Uuid) -> sqlx::Result<Vec<RuleRow>> {
    sqlx::query_as(
        "select id, subject_id, requester_id, purpose, persona_id, max_sensitivity, \
                allow_keys, priority, created_seq, created_at \
           from disclosure_rules \
          where subject_id = $1 \
          order by created_seq desc",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await
}

/// Exact matches plus every applicable wildcard — the engine ranks them.
pub async fn list_candidates(
    pool: &PgPool,
    subject_id: Uuid,
    requester_id: Uuid,
    purpose: &str,
) -> sqlx::Result<Vec<RuleRow>> {
    sqlx::query_as(
        "select id, subject_id, requester_id, purpose, persona_id, max_sensitivity, \
                allow_keys, priority, created_seq, created_at \
           from disclosure_rules \
          where subject_id = $1 \
            and (requester_id = $2 or requester_id is null) \
            and (purpose = $3 or purpose is null)",
    )
    .bind(subject_id)
    .bind(requester_id)
    .bind(purpose)
    .fetch_all(pool)
    .await
}

pub struct NewRule<'a> {
    pub subject_id: Uuid,
    pub requester_id: Option<Uuid>,
    pub purpose: Option<&'a str>,
    pub persona_id: Uuid,
    pub max_sensitivity: i16,
    pub allow_keys: Option<&'a [String]>,
    pub priority: i32,
}

pub async fn insert_rule(pool: &PgPool, rule: NewRule<'_>) -> sqlx::Result<RuleRow> {
    sqlx::query_as(
        "insert into disclosure_rules \
            (id, subject_id, requester_id, purpose, persona_id, max_sensitivity, allow_keys, priority) \
         values ($1, $2, $3, $4, $5, $6, $7, $8) \
         returning id, subject_id, requester_id, purpose, persona_id, max_sensitivity, \
                   allow_keys, priority, created_seq, created_at",
    )
    .bind(afixo_common::ids::new_id())
    .bind(rule.subject_id)
    .bind(rule.requester_id)
    .bind(rule.purpose)
    .bind(rule.persona_id)
    .bind(rule.max_sensitivity)
    .bind(rule.allow_keys)
    .bind(rule.priority)
    .fetch_one(pool)
    .await
}

/// Returns whether a row was deleted. Ownership is part of the predicate, so a
/// foreign rule id is indistinguishable from a missing one.
pub async fn delete_rule(pool: &PgPool, subject_id: Uuid, rule_id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query("delete from disclosure_rules where id = $1 and subject_id = $2")
        .bind(rule_id)
        .bind(subject_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}
