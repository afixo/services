//! Postgres access for identity-service. Runtime-checked queries (`query_as`)
//! so the workspace builds without a database; switch to `query!` macros with
//! `cargo sqlx prepare` if compile-time checking becomes worth the friction.
//!
//! sqlx 0.9 only accepts `&'static str` SQL (no `format!`), which is why the
//! column list is spelled out in every query instead of being shared.
//!
//! Ownership is part of the predicate of every mutating query
//! (`… and subject_id = $2`), so a foreign persona is indistinguishable from a
//! missing one: the caller learns "nothing happened", never "not yours".
//! Fields are always read `order by key collate "C"` — byte order, independent
//! of the database locale — so the engine and the UI see a stable order.

use std::{collections::HashMap, fmt};

use afixo_common::{ids, time::to_proto};
use afixo_proto::{Field, Persona, Subject};
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct SubjectRow {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub created_at: OffsetDateTime,
}

impl From<SubjectRow> for Subject {
    fn from(r: SubjectRow) -> Self {
        Self {
            id: r.id.to_string(),
            handle: r.handle,
            display_name: r.display_name,
            created_at: Some(to_proto(r.created_at)),
        }
    }
}

#[derive(Debug, FromRow)]
pub struct PersonaRow {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub label: String,
    pub created_at: OffsetDateTime,
}

/// One persona field. `value` is personal data: the `Debug` impl redacts it
/// so a stray `{:?}` can never put it in a log line (CLAUDE.md invariant 6).
#[derive(FromRow)]
pub struct FieldRow {
    pub persona_id: Uuid,
    pub key: String,
    pub value: String,
    pub sensitivity: i16,
}

impl fmt::Debug for FieldRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldRow")
            .field("persona_id", &self.persona_id)
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .field("sensitivity", &self.sensitivity)
            .finish()
    }
}

impl From<FieldRow> for Field {
    fn from(r: FieldRow) -> Self {
        Self {
            key: r.key,
            value: r.value,
            sensitivity: i32::from(r.sensitivity),
        }
    }
}

/// A persona with its fields ordered by key — the only shape a `Persona`
/// message is ever built from, so every response carries the fields.
#[derive(Debug)]
pub struct PersonaWithFields {
    pub persona: PersonaRow,
    pub fields: Vec<FieldRow>,
}

impl From<PersonaWithFields> for Persona {
    fn from(p: PersonaWithFields) -> Self {
        Self {
            id: p.persona.id.to_string(),
            subject_id: p.persona.subject_id.to_string(),
            label: p.persona.label,
            fields: p.fields.into_iter().map(Into::into).collect(),
            created_at: Some(to_proto(p.persona.created_at)),
        }
    }
}

// ---------------------------------------------------------------------------
// subjects

/// Idempotent on `handle`: a second call returns the existing row (same id,
/// same `created_at`) with the display name refreshed.
pub async fn ensure_subject(
    pool: &PgPool,
    handle: &str,
    display_name: &str,
) -> sqlx::Result<SubjectRow> {
    sqlx::query_as(
        "insert into subjects (id, handle, display_name) values ($1, $2, $3) \
         on conflict (handle) do update set display_name = excluded.display_name \
         returning id, handle, display_name, created_at",
    )
    .bind(ids::new_id())
    .bind(handle)
    .bind(display_name)
    .fetch_one(pool)
    .await
}

pub async fn get_subject(pool: &PgPool, subject_id: Uuid) -> sqlx::Result<Option<SubjectRow>> {
    sqlx::query_as("select id, handle, display_name, created_at from subjects where id = $1")
        .bind(subject_id)
        .fetch_optional(pool)
        .await
}

pub async fn get_subject_by_handle(
    pool: &PgPool,
    handle: &str,
) -> sqlx::Result<Option<SubjectRow>> {
    sqlx::query_as("select id, handle, display_name, created_at from subjects where handle = $1")
        .bind(handle)
        .fetch_optional(pool)
        .await
}

// ---------------------------------------------------------------------------
// personas

/// Every persona of a subject, oldest first, each with its fields.
pub async fn list_personas(
    pool: &PgPool,
    subject_id: Uuid,
) -> sqlx::Result<Vec<PersonaWithFields>> {
    let personas: Vec<PersonaRow> = sqlx::query_as(
        "select id, subject_id, label, created_at \
           from personas \
          where subject_id = $1 \
          order by created_at, id",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;
    if personas.is_empty() {
        return Ok(Vec::new());
    }

    let persona_ids: Vec<Uuid> = personas.iter().map(|p| p.id).collect();
    let rows: Vec<FieldRow> = sqlx::query_as(
        "select persona_id, key, value, sensitivity \
           from persona_fields \
          where persona_id = any($1) \
          order by key collate \"C\"",
    )
    .bind(persona_ids.as_slice())
    .fetch_all(pool)
    .await?;

    // Rows arrive sorted by key; pushing in that order keeps each group sorted.
    let mut by_persona: HashMap<Uuid, Vec<FieldRow>> = HashMap::new();
    for row in rows {
        by_persona.entry(row.persona_id).or_default().push(row);
    }
    Ok(personas
        .into_iter()
        .map(|persona| {
            let fields = by_persona.remove(&persona.id).unwrap_or_default();
            PersonaWithFields { persona, fields }
        })
        .collect())
}

/// A duplicate label surfaces as a unique violation (`ALREADY_EXISTS` via
/// `grpc::db_status`); an unknown subject as a foreign-key violation.
pub async fn insert_persona(
    pool: &PgPool,
    subject_id: Uuid,
    label: &str,
) -> sqlx::Result<PersonaRow> {
    sqlx::query_as(
        "insert into personas (id, subject_id, label) values ($1, $2, $3) \
         returning id, subject_id, label, created_at",
    )
    .bind(ids::new_id())
    .bind(subject_id)
    .bind(label)
    .fetch_one(pool)
    .await
}

/// With `subject_id` the persona must belong to that subject; `None` skips the
/// ownership filter (disclosure's read path, which compares `subject_id` itself).
pub async fn get_persona(
    pool: &PgPool,
    persona_id: Uuid,
    subject_id: Option<Uuid>,
) -> sqlx::Result<Option<PersonaWithFields>> {
    let persona: Option<PersonaRow> = sqlx::query_as(
        "select id, subject_id, label, created_at \
           from personas \
          where id = $1 and ($2::uuid is null or subject_id = $2)",
    )
    .bind(persona_id)
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    let Some(persona) = persona else {
        return Ok(None);
    };
    let fields = list_fields(pool, persona.id).await?;
    Ok(Some(PersonaWithFields { persona, fields }))
}

async fn list_fields(pool: &PgPool, persona_id: Uuid) -> sqlx::Result<Vec<FieldRow>> {
    sqlx::query_as(
        "select persona_id, key, value, sensitivity \
           from persona_fields \
          where persona_id = $1 \
          order by key collate \"C\"",
    )
    .bind(persona_id)
    .fetch_all(pool)
    .await
}

/// Returns whether a row was deleted (fields go with it, `on delete cascade`).
/// Ownership is part of the predicate, so a foreign persona id is
/// indistinguishable from a missing one.
pub async fn delete_persona(
    pool: &PgPool,
    subject_id: Uuid,
    persona_id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query("delete from personas where id = $1 and subject_id = $2")
        .bind(persona_id)
        .bind(subject_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

// ---------------------------------------------------------------------------
// fields

/// Insert or update one field. The row to insert is selected from the
/// subject's own personas, so nothing is written for a foreign or missing
/// persona and the function returns `false`.
pub async fn upsert_field(
    pool: &PgPool,
    subject_id: Uuid,
    persona_id: Uuid,
    key: &str,
    value: &str,
    sensitivity: i16,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "insert into persona_fields (persona_id, key, value, sensitivity) \
         select id, $3::text, $4::text, $5::smallint \
           from personas \
          where id = $1 and subject_id = $2 \
         on conflict (persona_id, key) do update \
            set value = excluded.value, \
                sensitivity = excluded.sensitivity, \
                updated_at = now()",
    )
    .bind(persona_id)
    .bind(subject_id)
    .bind(key)
    .bind(value)
    .bind(sensitivity)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Returns whether a field was deleted. `false` covers an absent key and a
/// foreign or missing persona alike.
pub async fn delete_field(
    pool: &PgPool,
    subject_id: Uuid,
    persona_id: Uuid,
    key: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "delete from persona_fields \
          where key = $3 \
            and persona_id in (select id from personas where id = $1 and subject_id = $2)",
    )
    .bind(persona_id)
    .bind(subject_id)
    .bind(key)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}
