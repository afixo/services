//! gRPC surface of disclosure-service.
//!
//! STATUS: skeleton. `Disclose` answers UNIMPLEMENTED; the orchestration it
//! must perform is fixed (CLAUDE.md) and the engine it calls is finished and
//! property-tested (`crates/engine`).

use afixo_proto::{
    DiscloseRequest, DiscloseResponse, audit_service_client::AuditServiceClient,
    disclosure_service_server::DisclosureService, identity_service_client::IdentityServiceClient,
    policy_service_client::PolicyServiceClient,
};
use tonic::{Request, Response, Status, transport::Channel};

#[derive(Debug, Clone)]
pub struct DisclosureSvc {
    #[allow(dead_code)]
    identity: IdentityServiceClient<Channel>,
    #[allow(dead_code)]
    policy: PolicyServiceClient<Channel>,
    #[allow(dead_code)]
    audit: AuditServiceClient<Channel>,
}

impl DisclosureSvc {
    pub fn new(identity: Channel, policy: Channel, audit: Channel) -> Self {
        Self {
            identity: IdentityServiceClient::new(identity),
            policy: PolicyServiceClient::new(policy),
            audit: AuditServiceClient::new(audit),
        }
    }
}

#[tonic::async_trait]
impl DisclosureService for DisclosureSvc {
    async fn disclose(
        &self,
        _req: Request<DiscloseRequest>,
    ) -> Result<Response<DiscloseResponse>, Status> {
        Err(Status::unimplemented(
            "disclosure.Disclose: not implemented yet",
        ))
    }
}
