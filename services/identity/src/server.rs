//! gRPC surface of identity-service.
//!
//! STATUS: skeleton. Every RPC answers UNIMPLEMENTED until implemented; the
//! schema (migrations/) and the contract (proto/afixo/v1/identity.proto) are
//! final. Implementation plan is in CLAUDE.md.

use afixo_proto::{
    CreatePersonaRequest, DeleteFieldRequest, DeletePersonaRequest, DeletePersonaResponse,
    EnsureSubjectRequest, GetPersonaRequest, GetSubjectRequest, ListPersonasRequest,
    ListPersonasResponse, Persona, ResolveSubjectRequest, Subject, UpsertFieldRequest,
    identity_service_server::IdentityService,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

#[derive(Debug, Clone)]
pub struct IdentitySvc {
    #[allow(dead_code)] // used once the RPCs are implemented
    pool: PgPool,
}

impl IdentitySvc {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn todo(rpc: &'static str) -> Status {
    Status::unimplemented(format!("identity.{rpc}: not implemented yet"))
}

#[tonic::async_trait]
impl IdentityService for IdentitySvc {
    async fn ensure_subject(
        &self,
        _req: Request<EnsureSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        Err(todo("EnsureSubject"))
    }

    async fn get_subject(
        &self,
        _req: Request<GetSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        Err(todo("GetSubject"))
    }

    async fn resolve_subject(
        &self,
        _req: Request<ResolveSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        Err(todo("ResolveSubject"))
    }

    async fn list_personas(
        &self,
        _req: Request<ListPersonasRequest>,
    ) -> Result<Response<ListPersonasResponse>, Status> {
        Err(todo("ListPersonas"))
    }

    async fn create_persona(
        &self,
        _req: Request<CreatePersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("CreatePersona"))
    }

    async fn get_persona(
        &self,
        _req: Request<GetPersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("GetPersona"))
    }

    async fn delete_persona(
        &self,
        _req: Request<DeletePersonaRequest>,
    ) -> Result<Response<DeletePersonaResponse>, Status> {
        Err(todo("DeletePersona"))
    }

    async fn upsert_field(
        &self,
        _req: Request<UpsertFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("UpsertField"))
    }

    async fn delete_field(
        &self,
        _req: Request<DeleteFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("DeleteField"))
    }
}
