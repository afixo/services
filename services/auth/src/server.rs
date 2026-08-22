//! gRPC surface of auth-service. Two caller types meet here and are never the
//! same principal: subjects (GitHub login, rotating sessions) and requesters
//! (client credentials, opaque tokens). See CLAUDE.md for the invariants.

use afixo_common::{
    grpc, ids,
    time::{now, to_proto},
    tokens,
};
use afixo_proto::{
    BeginGithubLoginRequest, BeginGithubLoginResponse, CompleteGithubLoginRequest,
    CreateRequesterRequest, CreateRequesterResponse, EnsureSubjectRequest, GetRequesterRequest,
    GetSubjectRequest, IntrospectTokenRequest, IntrospectTokenResponse, IssueClientTokenRequest,
    IssueClientTokenResponse, ListRequestersRequest, ListRequestersResponse, RefreshSessionRequest,
    Requester, ResolveSubjectRequest, RevokeSessionRequest, RevokeSessionResponse,
    RotateRequesterSecretRequest, RotateRequesterSecretResponse, SessionTokens, Subject,
    auth_service_server::AuthService, identity_service_client::IdentityServiceClient,
    introspect_token_response::Principal,
};
use sqlx::PgPool;
use tonic::{Code, Request, Response, Status, transport::Channel};
use uuid::Uuid;

use crate::{
    github::{Github, GithubError, GithubUser},
    repo::{self, NewSession, RequesterRow, SessionRow},
    settings::{ACCESS_TTL, CLIENT_TOKEN_TTL, REFRESH_TTL, STATE_TTL},
};

const ROLE_SUBJECT: &str = "subject";
const DEFAULT_REDIRECT: &str = "/app";
const NAME_MAX: usize = 80;

#[derive(Debug, Clone)]
pub struct AuthSvc {
    pool: PgPool,
    identity: IdentityServiceClient<Channel>,
    github: Github,
}

struct Minted {
    row: SessionRow,
    access: String,
    refresh: String,
}

impl AuthSvc {
    pub fn new(pool: PgPool, identity: Channel, github: Github) -> Self {
        Self {
            pool,
            identity: IdentityServiceClient::new(identity),
            github,
        }
    }

    /// A fresh access/refresh pair for `subject_id`. `rotate_from` marks the
    /// consumed session as rotated in the same transaction; `family` keeps the
    /// pair in an existing family (None = a new login, new family).
    async fn mint(
        &self,
        subject_id: Uuid,
        family: Option<Uuid>,
        rotate_from: Option<Uuid>,
    ) -> Result<Minted, Status> {
        let access = tokens::random_token();
        let refresh = tokens::random_token();
        let at = now();
        let next = NewSession {
            subject_id,
            family_id: family.unwrap_or_else(ids::new_id),
            access_hash: tokens::sha256(&access),
            refresh_hash: tokens::sha256(&refresh),
            access_expires_at: at + ACCESS_TTL,
            refresh_expires_at: at + REFRESH_TTL,
        };
        let row = match rotate_from {
            Some(old) => repo::rotate_session(&self.pool, old, &next).await,
            None => repo::insert_session(&self.pool, &next).await,
        }
        .map_err(|e| grpc::db_status("write session", e))?;
        Ok(Minted {
            row,
            access,
            refresh,
        })
    }

    async fn session_tokens(
        &self,
        minted: Minted,
        redirect_to: String,
    ) -> Result<SessionTokens, Status> {
        let subject = self
            .identity
            .clone()
            .get_subject(GetSubjectRequest {
                subject_id: minted.row.subject_id.to_string(),
            })
            .await
            .map_err(|s| grpc::internal("identity.GetSubject", s))?
            .into_inner();
        Ok(SessionTokens {
            access_token: minted.access,
            refresh_token: minted.refresh,
            access_expires_at: Some(to_proto(minted.row.access_expires_at)),
            refresh_expires_at: Some(to_proto(minted.row.refresh_expires_at)),
            subject: Some(subject),
            roles: vec![ROLE_SUBJECT.to_owned()],
            redirect_to,
        })
    }

    /// The subject behind a GitHub account. The stored link wins (a renamed
    /// GitHub login keeps its personas); a new account gets a new subject and
    /// never an existing subject's handle.
    async fn subject_for(&self, user: &GithubUser) -> Result<Uuid, Status> {
        if let Some(link) = repo::find_identity(&self.pool, user.id)
            .await
            .map_err(|e| grpc::db_status("find identity", e))?
        {
            if link.login != user.login {
                repo::touch_identity(&self.pool, user.id, &user.login)
                    .await
                    .map_err(|e| grpc::db_status("touch identity", e))?;
            }
            return Ok(link.subject_id);
        }

        let handle = self.free_handle(&user.login.to_lowercase()).await?;
        let display_name = user
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| user.login.clone());
        let subject: Subject = self
            .identity
            .clone()
            .ensure_subject(EnsureSubjectRequest {
                handle,
                display_name,
            })
            .await
            .map_err(|s| grpc::internal("identity.EnsureSubject", s))?
            .into_inner();
        let subject_id = ids::parse("subject.id", &subject.id)?;
        repo::link_identity(&self.pool, user.id, subject_id, &user.login)
            .await
            .map_err(|e| grpc::db_status("link identity", e))?;
        tracing::info!(%subject_id, github_id = user.id, "subject created from github login");
        Ok(subject_id)
    }

    /// `login` unless a subject already owns it (someone else's old GitHub
    /// name, say) or it is not a valid handle — then `<login>-<4 hex>`.
    async fn free_handle(&self, login: &str) -> Result<String, Status> {
        let taken = match self
            .identity
            .clone()
            .resolve_subject(ResolveSubjectRequest {
                handle: login.to_owned(),
            })
            .await
        {
            Ok(_) => true,
            Err(s) if s.code() == Code::NotFound => false,
            Err(s) => return Err(grpc::internal("identity.ResolveSubject", s)),
        };
        if !taken && ids::validate_handle(login).is_ok() {
            return Ok(login.to_owned());
        }
        let suffix = tokens::random_client_id()[4..8].to_owned();
        let base: String = login
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .take(30)
            .collect();
        let base = base.trim_matches('-');
        let base = if base.len() < 2 { "user" } else { base };
        Ok(format!("{base}-{suffix}"))
    }

    async fn requester_for_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<RequesterRow, Status> {
        let requester = repo::requester_by_client_id(&self.pool, client_id)
            .await
            .map_err(|e| grpc::db_status("find requester", e))?;
        // Same answer and similar work whether the client id is unknown or the
        // secret is wrong: compare against a dummy digest when there is no row.
        let presented = tokens::sha256(client_secret);
        let stored = requester
            .as_ref()
            .map_or([0u8; 32].as_slice(), |r| r.secret_hash.as_slice());
        if requester.is_none() || !tokens::digest_eq(stored, &presented) {
            return Err(Status::unauthenticated("invalid_client"));
        }
        Ok(requester.expect("checked above"))
    }
}

/// Only an in-app absolute path may be a post-login destination.
fn validate_redirect(raw: &str) -> Result<String, Status> {
    if raw.is_empty() {
        return Ok(DEFAULT_REDIRECT.to_owned());
    }
    let ok =
        raw.starts_with('/') && !raw.starts_with("//") && !raw.contains('\\') && raw.len() <= 512;
    if ok {
        Ok(raw.to_owned())
    } else {
        Err(Status::invalid_argument(
            "redirect_to: must be an in-app path",
        ))
    }
}

fn validate_name(raw: &str) -> Result<&str, Status> {
    let name = raw.trim();
    if name.is_empty() || name.len() > NAME_MAX {
        return Err(Status::invalid_argument("name: 1..=80 characters"));
    }
    Ok(name)
}

fn requester_proto(r: &RequesterRow) -> Requester {
    Requester {
        id: r.id.to_string(),
        client_id: r.client_id.clone(),
        name: r.name.clone(),
        owner_subject_id: r.owner_subject_id.to_string(),
        created_at: Some(to_proto(r.created_at)),
        rotated_at: r.rotated_at.map(to_proto),
    }
}

#[tonic::async_trait]
impl AuthService for AuthSvc {
    async fn begin_github_login(
        &self,
        req: Request<BeginGithubLoginRequest>,
    ) -> Result<Response<BeginGithubLoginResponse>, Status> {
        let redirect_to = validate_redirect(&req.get_ref().redirect_to)?;
        let state = tokens::random_token();
        repo::insert_state(&self.pool, &state, &redirect_to, now() + STATE_TTL)
            .await
            .map_err(|e| grpc::db_status("insert state", e))?;
        Ok(Response::new(BeginGithubLoginResponse {
            authorize_url: self.github.authorize_url(&state),
        }))
    }

    async fn complete_github_login(
        &self,
        req: Request<CompleteGithubLoginRequest>,
    ) -> Result<Response<SessionTokens>, Status> {
        let req = req.into_inner();
        let redirect_to = repo::consume_state(&self.pool, &req.state)
            .await
            .map_err(|e| grpc::db_status("consume state", e))?
            .ok_or_else(|| Status::unauthenticated("invalid_state"))?;

        let gh_token = match self.github.exchange_code(&req.code).await {
            Ok(t) => t,
            Err(GithubError::Rejected(why)) => {
                tracing::info!(why, "github rejected the code");
                return Err(Status::unauthenticated("invalid_code"));
            }
            Err(e) => return Err(grpc::internal("github token exchange", e)),
        };
        let user = match self.github.fetch_user(&gh_token).await {
            Ok(u) => u,
            Err(GithubError::Rejected(why)) => {
                tracing::info!(why, "github rejected the token");
                return Err(Status::unauthenticated("invalid_code"));
            }
            Err(e) => return Err(grpc::internal("github user fetch", e)),
        };

        let subject_id = self.subject_for(&user).await?;
        let minted = self.mint(subject_id, None, None).await?;
        tracing::info!(%subject_id, family_id = %minted.row.family_id, "login");
        Ok(Response::new(
            self.session_tokens(minted, redirect_to).await?,
        ))
    }

    async fn refresh_session(
        &self,
        req: Request<RefreshSessionRequest>,
    ) -> Result<Response<SessionTokens>, Status> {
        let hash = tokens::sha256(&req.get_ref().refresh_token);
        let Some(session) = repo::session_by_refresh_hash(&self.pool, &hash)
            .await
            .map_err(|e| grpc::db_status("find session", e))?
        else {
            return Err(Status::unauthenticated("invalid_token"));
        };

        if session.revoked_at.is_some() {
            return Err(Status::unauthenticated("revoked"));
        }
        if session.rotated_at.is_some() {
            // Replay of a spent refresh token: treat as theft, kill the family.
            let revoked = repo::revoke_family(&self.pool, session.family_id)
                .await
                .map_err(|e| grpc::db_status("revoke family", e))?;
            tracing::warn!(
                family_id = %session.family_id,
                subject_id = %session.subject_id,
                revoked,
                "refresh token reuse detected; family revoked"
            );
            return Err(Status::unauthenticated("refresh_reuse"));
        }
        if session.refresh_expires_at <= now() {
            return Err(Status::unauthenticated("expired"));
        }

        let minted = self
            .mint(
                session.subject_id,
                Some(session.family_id),
                Some(session.id),
            )
            .await?;
        tracing::debug!(family_id = %session.family_id, "session rotated");
        Ok(Response::new(
            self.session_tokens(minted, DEFAULT_REDIRECT.to_owned())
                .await?,
        ))
    }

    async fn revoke_session(
        &self,
        req: Request<RevokeSessionRequest>,
    ) -> Result<Response<RevokeSessionResponse>, Status> {
        let hash = tokens::sha256(&req.get_ref().access_token);
        // Idempotent: an unknown or already-revoked token is not an error.
        repo::revoke_by_access_hash(&self.pool, &hash)
            .await
            .map_err(|e| grpc::db_status("revoke session", e))?;
        Ok(Response::new(RevokeSessionResponse {}))
    }

    async fn introspect_token(
        &self,
        req: Request<IntrospectTokenRequest>,
    ) -> Result<Response<IntrospectTokenResponse>, Status> {
        let token = &req.get_ref().token;
        if token.is_empty() {
            return Ok(Response::new(IntrospectTokenResponse::default()));
        }
        let hash = tokens::sha256(token);

        if let Some(session) = repo::active_session_by_access_hash(&self.pool, &hash)
            .await
            .map_err(|e| grpc::db_status("introspect session", e))?
        {
            return Ok(Response::new(IntrospectTokenResponse {
                active: true,
                principal: Some(Principal::SubjectId(session.subject_id.to_string())),
                expires_at: Some(to_proto(session.access_expires_at)),
            }));
        }
        if let Some((requester_id, expires_at)) = repo::live_token(&self.pool, &hash)
            .await
            .map_err(|e| grpc::db_status("introspect token", e))?
        {
            return Ok(Response::new(IntrospectTokenResponse {
                active: true,
                principal: Some(Principal::RequesterId(requester_id.to_string())),
                expires_at: Some(to_proto(expires_at)),
            }));
        }
        Ok(Response::new(IntrospectTokenResponse::default()))
    }

    async fn issue_client_token(
        &self,
        req: Request<IssueClientTokenRequest>,
    ) -> Result<Response<IssueClientTokenResponse>, Status> {
        let req = req.get_ref();
        let requester = self
            .requester_for_credentials(&req.client_id, &req.client_secret)
            .await?;
        let token = tokens::random_token();
        let expires_at = now() + CLIENT_TOKEN_TTL;
        repo::insert_token(
            &self.pool,
            &tokens::sha256(&token),
            requester.id,
            expires_at,
        )
        .await
        .map_err(|e| grpc::db_status("insert token", e))?;
        tracing::info!(requester_id = %requester.id, "client token issued");
        Ok(Response::new(IssueClientTokenResponse {
            access_token: token,
            expires_in_seconds: CLIENT_TOKEN_TTL.whole_seconds(),
        }))
    }

    async fn create_requester(
        &self,
        req: Request<CreateRequesterRequest>,
    ) -> Result<Response<CreateRequesterResponse>, Status> {
        let req = req.get_ref();
        let owner = ids::parse("owner_subject_id", &req.owner_subject_id)?;
        let name = validate_name(&req.name)?;
        let client_id = tokens::random_client_id();
        let secret = tokens::random_token();
        let row = repo::insert_requester(
            &self.pool,
            owner,
            &client_id,
            name,
            &tokens::sha256(&secret),
        )
        .await
        .map_err(|e| grpc::db_status("insert requester", e))?;
        tracing::info!(requester_id = %row.id, owner_subject_id = %owner, "requester created");
        Ok(Response::new(CreateRequesterResponse {
            requester: Some(requester_proto(&row)),
            client_secret: secret,
        }))
    }

    async fn list_requesters(
        &self,
        req: Request<ListRequestersRequest>,
    ) -> Result<Response<ListRequestersResponse>, Status> {
        let owner = ids::parse_opt(
            "owner_subject_id",
            req.get_ref().owner_subject_id.as_deref(),
        )?;
        let rows = repo::list_requesters(&self.pool, owner)
            .await
            .map_err(|e| grpc::db_status("list requesters", e))?;
        Ok(Response::new(ListRequestersResponse {
            requesters: rows.iter().map(requester_proto).collect(),
        }))
    }

    async fn get_requester(
        &self,
        req: Request<GetRequesterRequest>,
    ) -> Result<Response<Requester>, Status> {
        let id = ids::parse("requester_id", &req.get_ref().requester_id)?;
        let row = repo::requester_by_id(&self.pool, id)
            .await
            .map_err(|e| grpc::db_status("get requester", e))?
            .ok_or_else(|| Status::not_found("requester"))?;
        Ok(Response::new(requester_proto(&row)))
    }

    async fn rotate_requester_secret(
        &self,
        req: Request<RotateRequesterSecretRequest>,
    ) -> Result<Response<RotateRequesterSecretResponse>, Status> {
        let req = req.get_ref();
        let owner = ids::parse("owner_subject_id", &req.owner_subject_id)?;
        let id = ids::parse("requester_id", &req.requester_id)?;
        let secret = tokens::random_token();
        let rotated = repo::rotate_secret(&self.pool, id, owner, &tokens::sha256(&secret))
            .await
            .map_err(|e| grpc::db_status("rotate secret", e))?;
        if !rotated {
            // Foreign or missing: indistinguishable on purpose.
            return Err(Status::not_found("requester"));
        }
        tracing::info!(requester_id = %id, owner_subject_id = %owner, "requester secret rotated");
        Ok(Response::new(RotateRequesterSecretResponse {
            client_secret: secret,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects() {
        assert_eq!(validate_redirect("").unwrap(), "/app");
        assert_eq!(validate_redirect("/app/policies").unwrap(), "/app/policies");
        assert!(validate_redirect("//evil.test").is_err());
        assert!(validate_redirect("https://evil.test").is_err());
        assert!(validate_redirect("/a\\b").is_err());
    }

    #[test]
    fn names() {
        assert_eq!(validate_name("  ShopCo ").unwrap(), "ShopCo");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(81)).is_err());
    }
}
