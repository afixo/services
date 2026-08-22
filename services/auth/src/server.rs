//! gRPC surface of auth-service.
//!
//! STATUS: skeleton. Every RPC answers UNIMPLEMENTED until implemented; the
//! schema (migrations/) and the contract (proto/afixo/v1/auth.proto) are final.
//! Implementation plan and invariants are in CLAUDE.md.

use afixo_proto::{
    BeginGithubLoginRequest, BeginGithubLoginResponse, CompleteGithubLoginRequest,
    CreateRequesterRequest, CreateRequesterResponse, GetRequesterRequest, IntrospectTokenRequest,
    IntrospectTokenResponse, IssueClientTokenRequest, IssueClientTokenResponse,
    ListRequestersRequest, ListRequestersResponse, RefreshSessionRequest, Requester,
    RevokeSessionRequest, RevokeSessionResponse, RotateRequesterSecretRequest,
    RotateRequesterSecretResponse, SessionTokens, auth_service_server::AuthService,
    identity_service_client::IdentityServiceClient,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status, transport::Channel};

#[derive(Debug, Clone)]
pub struct AuthSvc {
    #[allow(dead_code)]
    pool: PgPool,
    #[allow(dead_code)]
    identity: IdentityServiceClient<Channel>,
}

impl AuthSvc {
    pub fn new(pool: PgPool, identity: Channel) -> Self {
        Self {
            pool,
            identity: IdentityServiceClient::new(identity),
        }
    }
}

fn todo(rpc: &'static str) -> Status {
    Status::unimplemented(format!("auth.{rpc}: not implemented yet"))
}

#[tonic::async_trait]
impl AuthService for AuthSvc {
    async fn begin_github_login(
        &self,
        _req: Request<BeginGithubLoginRequest>,
    ) -> Result<Response<BeginGithubLoginResponse>, Status> {
        Err(todo("BeginGithubLogin"))
    }

    async fn complete_github_login(
        &self,
        _req: Request<CompleteGithubLoginRequest>,
    ) -> Result<Response<SessionTokens>, Status> {
        Err(todo("CompleteGithubLogin"))
    }

    async fn refresh_session(
        &self,
        _req: Request<RefreshSessionRequest>,
    ) -> Result<Response<SessionTokens>, Status> {
        Err(todo("RefreshSession"))
    }

    async fn revoke_session(
        &self,
        _req: Request<RevokeSessionRequest>,
    ) -> Result<Response<RevokeSessionResponse>, Status> {
        Err(todo("RevokeSession"))
    }

    async fn introspect_token(
        &self,
        _req: Request<IntrospectTokenRequest>,
    ) -> Result<Response<IntrospectTokenResponse>, Status> {
        Err(todo("IntrospectToken"))
    }

    async fn issue_client_token(
        &self,
        _req: Request<IssueClientTokenRequest>,
    ) -> Result<Response<IssueClientTokenResponse>, Status> {
        Err(todo("IssueClientToken"))
    }

    async fn create_requester(
        &self,
        _req: Request<CreateRequesterRequest>,
    ) -> Result<Response<CreateRequesterResponse>, Status> {
        Err(todo("CreateRequester"))
    }

    async fn list_requesters(
        &self,
        _req: Request<ListRequestersRequest>,
    ) -> Result<Response<ListRequestersResponse>, Status> {
        Err(todo("ListRequesters"))
    }

    async fn get_requester(
        &self,
        _req: Request<GetRequesterRequest>,
    ) -> Result<Response<Requester>, Status> {
        Err(todo("GetRequester"))
    }

    async fn rotate_requester_secret(
        &self,
        _req: Request<RotateRequesterSecretRequest>,
    ) -> Result<Response<RotateRequesterSecretResponse>, Status> {
        Err(todo("RotateRequesterSecret"))
    }
}
