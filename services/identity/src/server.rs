//! gRPC surface of identity-service.
//!
//! Input validation lives here, at the boundary; ownership is enforced in
//! `repo.rs` by the SQL predicates. Every mutating RPC re-checks that the
//! persona belongs to the acting `subject_id` and answers `NOT_FOUND`
//! otherwise — the same answer as for a persona that does not exist.

use afixo_common::{grpc, ids};
use afixo_engine::Sensitivity;
use afixo_proto::{
    CreatePersonaRequest, DeleteFieldRequest, DeletePersonaRequest, DeletePersonaResponse,
    EnsureSubjectRequest, GetPersonaRequest, GetSubjectRequest, ListPersonasRequest,
    ListPersonasResponse, Persona, ResolveSubjectRequest, Subject, UpsertFieldRequest,
    identity_service_server::IdentityService,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::repo;

/// Labels are short display names (`legal`, `work`, `social`…).
const LABEL_MAX_CHARS: usize = 40;
/// Keys are identifiers: `[A-Za-z0-9_.-]{1,64}`.
const KEY_MAX_BYTES: usize = 64;
/// Values are bounded so a persona stays small enough to disclose in one reply.
const VALUE_MAX_CHARS: usize = 4096;

#[derive(Debug, Clone)]
pub struct IdentitySvc {
    pool: PgPool,
}

impl IdentitySvc {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The persona as its owner must see it after a mutation — scoped by
    /// `subject_id`, fields ordered by key.
    async fn owned_persona(
        &self,
        subject_id: Uuid,
        persona_id: Uuid,
    ) -> Result<Response<Persona>, Status> {
        repo::get_persona(&self.pool, persona_id, Some(subject_id))
            .await
            .map_err(|e| grpc::db_status("get persona", e))?
            .map(|p| Response::new(p.into()))
            .ok_or_else(|| Status::not_found("persona"))
    }
}

/// Trimmed, 1..=40 characters.
fn validate_label(raw: &str) -> Result<&str, Status> {
    let label = raw.trim();
    if (1..=LABEL_MAX_CHARS).contains(&label.chars().count()) {
        Ok(label)
    } else {
        Err(Status::invalid_argument(format!(
            "label: must be 1 to {LABEL_MAX_CHARS} characters"
        )))
    }
}

fn validate_key(key: &str) -> Result<(), Status> {
    let ok = (1..=KEY_MAX_BYTES).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "field.key: must match [A-Za-z0-9_.-]{1,64}",
        ))
    }
}

fn validate_value(value: &str) -> Result<(), Status> {
    if value.chars().count() <= VALUE_MAX_CHARS {
        Ok(())
    } else {
        Err(Status::invalid_argument(format!(
            "field.value: at most {VALUE_MAX_CHARS} characters"
        )))
    }
}

#[tonic::async_trait]
impl IdentityService for IdentitySvc {
    async fn ensure_subject(
        &self,
        req: Request<EnsureSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        let req = req.get_ref();
        let handle = req.handle.trim().to_ascii_lowercase();
        ids::validate_handle(&handle)?;
        let row = repo::ensure_subject(&self.pool, &handle, req.display_name.trim())
            .await
            .map_err(|e| grpc::db_status("ensure subject", e))?;
        tracing::info!(subject_id = %row.id, "subject ensured");
        Ok(Response::new(row.into()))
    }

    async fn get_subject(
        &self,
        req: Request<GetSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        let subject_id = ids::parse("subject_id", &req.get_ref().subject_id)?;
        let row = repo::get_subject(&self.pool, subject_id)
            .await
            .map_err(|e| grpc::db_status("get subject", e))?
            .ok_or_else(|| Status::not_found("subject"))?;
        Ok(Response::new(row.into()))
    }

    async fn resolve_subject(
        &self,
        req: Request<ResolveSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        // A handle that cannot exist is simply absent. disclosure turns
        // NOT_FOUND into the uniform deny, so a malformed handle must not get a
        // different answer than an unknown one.
        let handle = req.get_ref().handle.trim().to_ascii_lowercase();
        if ids::validate_handle(&handle).is_err() {
            return Err(Status::not_found("subject"));
        }
        let row = repo::get_subject_by_handle(&self.pool, &handle)
            .await
            .map_err(|e| grpc::db_status("resolve subject", e))?
            .ok_or_else(|| Status::not_found("subject"))?;
        Ok(Response::new(row.into()))
    }

    async fn list_personas(
        &self,
        req: Request<ListPersonasRequest>,
    ) -> Result<Response<ListPersonasResponse>, Status> {
        let subject_id = ids::parse("subject_id", &req.get_ref().subject_id)?;
        let personas = repo::list_personas(&self.pool, subject_id)
            .await
            .map_err(|e| grpc::db_status("list personas", e))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(Response::new(ListPersonasResponse { personas }))
    }

    async fn create_persona(
        &self,
        req: Request<CreatePersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let label = validate_label(&req.label)?;
        let persona = repo::insert_persona(&self.pool, subject_id, label)
            .await
            .map_err(|e| grpc::db_status("create persona", e))?;
        tracing::info!(persona_id = %persona.id, %subject_id, "persona created");
        let created = repo::PersonaWithFields {
            persona,
            fields: Vec::new(),
        };
        Ok(Response::new(created.into()))
    }

    async fn get_persona(
        &self,
        req: Request<GetPersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        let req = req.get_ref();
        let persona_id = ids::parse("persona_id", &req.persona_id)?;
        // Presence is the contract: `Some("")` is a malformed scope, not the
        // absence of one, so it fails closed instead of skipping the check.
        let subject_id = req
            .subject_id
            .as_deref()
            .map(|s| ids::parse("subject_id", s))
            .transpose()?;
        let persona = repo::get_persona(&self.pool, persona_id, subject_id)
            .await
            .map_err(|e| grpc::db_status("get persona", e))?
            .ok_or_else(|| Status::not_found("persona"))?;
        Ok(Response::new(persona.into()))
    }

    async fn delete_persona(
        &self,
        req: Request<DeletePersonaRequest>,
    ) -> Result<Response<DeletePersonaResponse>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let persona_id = ids::parse("persona_id", &req.persona_id)?;
        let deleted = repo::delete_persona(&self.pool, subject_id, persona_id)
            .await
            .map_err(|e| grpc::db_status("delete persona", e))?;
        if deleted {
            tracing::info!(%persona_id, %subject_id, "persona deleted");
            Ok(Response::new(DeletePersonaResponse {}))
        } else {
            Err(Status::not_found("persona"))
        }
    }

    async fn upsert_field(
        &self,
        req: Request<UpsertFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let persona_id = ids::parse("persona_id", &req.persona_id)?;
        let field = req
            .field
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("field: required"))?;
        validate_key(&field.key)?;
        validate_value(&field.value)?;
        // Clamped at the boundary: the CHECK constraint can never be hit by a
        // client value and an out-of-range tier can never be stored.
        let sensitivity = Sensitivity::from_i32_clamped(field.sensitivity).as_i16();

        let written = repo::upsert_field(
            &self.pool,
            subject_id,
            persona_id,
            &field.key,
            &field.value,
            sensitivity,
        )
        .await
        .map_err(|e| grpc::db_status("upsert field", e))?;
        if !written {
            return Err(Status::not_found("persona"));
        }
        tracing::info!(%persona_id, %subject_id, "field upserted");
        self.owned_persona(subject_id, persona_id).await
    }

    async fn delete_field(
        &self,
        req: Request<DeleteFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        let req = req.get_ref();
        let subject_id = ids::parse("subject_id", &req.subject_id)?;
        let persona_id = ids::parse("persona_id", &req.persona_id)?;
        let deleted = repo::delete_field(&self.pool, subject_id, persona_id, &req.key)
            .await
            .map_err(|e| grpc::db_status("delete field", e))?;
        if !deleted {
            return Err(Status::not_found("field"));
        }
        tracing::info!(%persona_id, %subject_id, "field deleted");
        self.owned_persona(subject_id, persona_id).await
    }
}
