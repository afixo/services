//! Shared handler state: one lazily-connected client per upstream service.

use afixo_common::{config, grpc};
use afixo_proto::{
    audit_service_client::AuditServiceClient, auth_service_client::AuthServiceClient,
    disclosure_service_client::DisclosureServiceClient,
    identity_service_client::IdentityServiceClient, policy_service_client::PolicyServiceClient,
};
use tonic::transport::Channel;

#[derive(Debug, Clone)]
pub struct AppState {
    pub auth: AuthServiceClient<Channel>,
    pub identity: IdentityServiceClient<Channel>,
    pub policy: PolicyServiceClient<Channel>,
    pub disclosure: DisclosureServiceClient<Channel>,
    pub audit: AuditServiceClient<Channel>,
    /// Origins allowed to call the machine listener from a browser (the dashboard's Explorer).
    pub cors_origins: Vec<String>,
}

impl AppState {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut cors_origins = config::list("CORS_ALLOWED_ORIGINS");
        if cors_origins.is_empty() {
            cors_origins.push("https://afixo.io".to_owned());
        }
        Ok(Self {
            auth: AuthServiceClient::new(grpc::channel("AUTH_URL")?),
            identity: IdentityServiceClient::new(grpc::channel("IDENTITY_URL")?),
            policy: PolicyServiceClient::new(grpc::channel("POLICY_URL")?),
            disclosure: DisclosureServiceClient::new(grpc::channel("DISCLOSURE_URL")?),
            audit: AuditServiceClient::new(grpc::channel("AUDIT_URL")?),
            cors_origins,
        })
    }
}
