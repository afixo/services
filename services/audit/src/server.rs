//! gRPC surface of audit-service.
//!
//! STATUS: skeleton — every RPC answers UNIMPLEMENTED. `chain.rs` (the hash
//! function) is complete and unit-tested; the append transaction, the list
//! query and the verifier are described in CLAUDE.md.

use afixo_proto::{
    ListDecisionsRequest, ListDecisionsResponse, RecordDecisionRequest, RecordDecisionResponse,
    VerifyChainRequest, VerifyChainResponse, audit_service_server::AuditService,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

#[derive(Debug, Clone)]
pub struct AuditSvc {
    #[allow(dead_code)]
    pool: PgPool,
}

impl AuditSvc {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn todo(rpc: &'static str) -> Status {
    Status::unimplemented(format!("audit.{rpc}: not implemented yet"))
}

#[tonic::async_trait]
impl AuditService for AuditSvc {
    async fn record(
        &self,
        _req: Request<RecordDecisionRequest>,
    ) -> Result<Response<RecordDecisionResponse>, Status> {
        Err(todo("Record"))
    }

    async fn list_decisions(
        &self,
        _req: Request<ListDecisionsRequest>,
    ) -> Result<Response<ListDecisionsResponse>, Status> {
        Err(todo("ListDecisions"))
    }

    async fn verify_chain(
        &self,
        _req: Request<VerifyChainRequest>,
    ) -> Result<Response<VerifyChainResponse>, Status> {
        Err(todo("VerifyChain"))
    }
}
