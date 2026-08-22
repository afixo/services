//! Integration tests: a real Postgres (fresh database per test via
//! `#[sqlx::test]`, `DATABASE_URL`), an in-process stand-in for GitHub (axum)
//! and an in-memory `IdentityService` (tonic) — the flows are exercised
//! end to end through the `AuthService` trait.

#![allow(clippy::unwrap_used, clippy::missing_panics_doc, dead_code)]

#[path = "../src/github.rs"]
mod github;
#[path = "../src/repo.rs"]
mod repo;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/settings.rs"]
mod settings;

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use afixo_proto::{
    BeginGithubLoginRequest, CompleteGithubLoginRequest, CreatePersonaRequest,
    CreateRequesterRequest, DeleteFieldRequest, DeletePersonaRequest, DeletePersonaResponse,
    EnsureSubjectRequest, GetPersonaRequest, GetRequesterRequest, GetSubjectRequest,
    IntrospectTokenRequest, IssueClientTokenRequest, ListPersonasRequest, ListPersonasResponse,
    ListRequestersRequest, Persona, RefreshSessionRequest, ResolveSubjectRequest,
    RevokeSessionRequest, RotateRequesterSecretRequest, SessionTokens, Subject, UpsertFieldRequest,
    auth_service_server::AuthService,
    identity_service_server::{IdentityService, IdentityServiceServer},
    introspect_token_response::Principal,
};
use axum::{
    Form, Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Code, Request, Response, Status, transport::Channel};

use github::Github;
use server::AuthSvc;
use settings::GithubConfig;

// ---------------------------------------------------------------------------
// GitHub stand-in: `code = good:<login>:<id>` is exchangeable, anything else is rejected.

#[derive(Default)]
struct FakeGithub {
    tokens: Mutex<HashMap<String, (i64, String)>>, // token → (id, login)
}

async fn gh_token(
    State(gh): State<Arc<FakeGithub>>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    assert_eq!(form.get("client_id").map(String::as_str), Some("cid"));
    assert_eq!(form.get("client_secret").map(String::as_str), Some("sec"));
    let code = form.get("code").cloned().unwrap_or_default();
    match code.strip_prefix("good:") {
        Some(rest) => {
            let (login, id) = rest.split_once(':').unwrap();
            let token = format!("gh_{login}_{id}");
            gh.tokens
                .lock()
                .unwrap()
                .insert(token.clone(), (id.parse().unwrap(), login.to_owned()));
            Json(json!({ "access_token": token, "token_type": "bearer" }))
        }
        None => Json(json!({ "error": "bad_verification_code", "error_description": "nope" })),
    }
}

async fn gh_user(
    State(gh): State<Arc<FakeGithub>>,
    headers: HeaderMap,
) -> (axum::http::StatusCode, Json<Value>) {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    match gh.tokens.lock().unwrap().get(bearer) {
        Some((id, login)) => (
            axum::http::StatusCode::OK,
            Json(json!({ "id": id, "login": login, "name": format!("{login} Example") })),
        ),
        None => (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        ),
    }
}

async fn start_fake_github() -> String {
    let state = Arc::new(FakeGithub::default());
    let app = Router::new()
        .route("/login/oauth/access_token", post(gh_token))
        .route("/user", get(gh_user))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// identity stand-in: subjects in memory, keyed by handle.

#[derive(Default)]
struct FakeIdentity {
    subjects: Mutex<Vec<Subject>>,
}

fn todo(what: &str) -> Status {
    Status::unimplemented(what.to_owned())
}

#[derive(Clone)]
struct IdentityStub(Arc<FakeIdentity>);

#[tonic::async_trait]
impl IdentityService for IdentityStub {
    async fn ensure_subject(
        &self,
        req: Request<EnsureSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        let req = req.into_inner();
        let mut subjects = self.0.subjects.lock().unwrap();
        if let Some(s) = subjects.iter().find(|s| s.handle == req.handle) {
            return Ok(Response::new(s.clone()));
        }
        let s = Subject {
            id: uuid::Uuid::now_v7().to_string(),
            handle: req.handle,
            display_name: req.display_name,
            created_at: None,
        };
        subjects.push(s.clone());
        Ok(Response::new(s))
    }
    async fn get_subject(
        &self,
        req: Request<GetSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        self.0
            .subjects
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == req.get_ref().subject_id)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("subject"))
    }
    async fn resolve_subject(
        &self,
        req: Request<ResolveSubjectRequest>,
    ) -> Result<Response<Subject>, Status> {
        self.0
            .subjects
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.handle == req.get_ref().handle)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found("subject"))
    }
    async fn list_personas(
        &self,
        _: Request<ListPersonasRequest>,
    ) -> Result<Response<ListPersonasResponse>, Status> {
        Err(todo("ListPersonas"))
    }
    async fn create_persona(
        &self,
        _: Request<CreatePersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("CreatePersona"))
    }
    async fn get_persona(
        &self,
        _: Request<GetPersonaRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("GetPersona"))
    }
    async fn delete_persona(
        &self,
        _: Request<DeletePersonaRequest>,
    ) -> Result<Response<DeletePersonaResponse>, Status> {
        Err(todo("DeletePersona"))
    }
    async fn upsert_field(
        &self,
        _: Request<UpsertFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("UpsertField"))
    }
    async fn delete_field(
        &self,
        _: Request<DeleteFieldRequest>,
    ) -> Result<Response<Persona>, Status> {
        Err(todo("DeleteField"))
    }
}

async fn start_fake_identity() -> (Channel, Arc<FakeIdentity>) {
    let fake = Arc::new(FakeIdentity::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let svc = IdentityServiceServer::new(IdentityStub(Arc::clone(&fake)));
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(svc)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy();
    (channel, fake)
}

// ---------------------------------------------------------------------------

async fn service(pool: PgPool) -> (AuthSvc, Arc<FakeIdentity>) {
    let gh = start_fake_github().await;
    let (identity, fake) = start_fake_identity().await;
    let github = Github::new(GithubConfig {
        client_id: "cid".into(),
        client_secret: "sec".into(),
        redirect_uri: "http://localhost:4321/api/v1/auth/github/callback".into(),
        oauth_base: gh.clone(),
        api_base: gh,
    });
    (AuthSvc::new(pool, identity, github), fake)
}

fn state_from(url: &str) -> String {
    url.split('&')
        .find_map(|kv| kv.strip_prefix("state="))
        .unwrap()
        .to_owned()
}

/// Full login: begin → (browser) → complete. Returns the session tokens.
async fn login(svc: &AuthSvc, login: &str, github_id: i64, redirect_to: &str) -> SessionTokens {
    let begin = svc
        .begin_github_login(Request::new(BeginGithubLoginRequest {
            redirect_to: redirect_to.to_owned(),
        }))
        .await
        .unwrap()
        .into_inner();
    let state = state_from(&begin.authorize_url);
    svc.complete_github_login(Request::new(CompleteGithubLoginRequest {
        code: format!("good:{login}:{github_id}"),
        state,
    }))
    .await
    .unwrap()
    .into_inner()
}

async fn introspect(svc: &AuthSvc, token: &str) -> Option<Principal> {
    let r = svc
        .introspect_token(Request::new(IntrospectTokenRequest {
            token: token.to_owned(),
        }))
        .await
        .unwrap()
        .into_inner();
    if r.active { r.principal } else { None }
}

fn code_of<T>(r: Result<T, Status>) -> Code {
    match r {
        Ok(_) => Code::Ok,
        Err(s) => s.code(),
    }
}

// ---------------------------------------------------------------------------
// subjects

#[sqlx::test(migrations = "./migrations")]
async fn login_creates_subject_and_session(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let tokens = login(&svc, "Alice", 42, "/app/policies").await;

    let subject = tokens.subject.unwrap();
    assert_eq!(subject.handle, "alice", "handles are lower-cased logins");
    assert_eq!(subject.display_name, "Alice Example");
    assert_eq!(tokens.roles, ["subject"]);
    assert_eq!(tokens.redirect_to, "/app/policies");
    assert_eq!(tokens.access_token.len(), 43);
    assert_ne!(tokens.access_token, tokens.refresh_token);

    match introspect(&svc, &tokens.access_token).await {
        Some(Principal::SubjectId(id)) => assert_eq!(id, subject.id),
        other => panic!("expected subject principal, got {other:?}"),
    }
    assert!(
        introspect(&svc, &tokens.refresh_token).await.is_none(),
        "a refresh token is not a bearer"
    );
    assert!(introspect(&svc, "nope").await.is_none());
    assert!(introspect(&svc, "").await.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn state_is_single_use_and_bad_codes_are_rejected(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let begin = svc
        .begin_github_login(Request::new(BeginGithubLoginRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let state = state_from(&begin.authorize_url);

    let bad = svc
        .complete_github_login(Request::new(CompleteGithubLoginRequest {
            code: "bad".into(),
            state: state.clone(),
        }))
        .await;
    assert_eq!(
        code_of(bad),
        Code::Unauthenticated,
        "github rejected the code"
    );

    // The state was consumed by the failed attempt: it cannot be replayed.
    let replay = svc
        .complete_github_login(Request::new(CompleteGithubLoginRequest {
            code: "good:bob:7".into(),
            state,
        }))
        .await;
    assert_eq!(code_of(replay), Code::Unauthenticated);

    let unknown = svc
        .complete_github_login(Request::new(CompleteGithubLoginRequest {
            code: "good:bob:7".into(),
            state: "x".into(),
        }))
        .await;
    assert_eq!(code_of(unknown), Code::Unauthenticated);

    let bad_redirect = svc
        .begin_github_login(Request::new(BeginGithubLoginRequest {
            redirect_to: "//evil.test".into(),
        }))
        .await;
    assert_eq!(code_of(bad_redirect), Code::InvalidArgument);
}

#[sqlx::test(migrations = "./migrations")]
async fn returning_user_keeps_subject_even_after_github_rename(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let first = login(&svc, "carol", 100, "").await;
    let again = login(&svc, "carol-renamed", 100, "").await;
    assert_eq!(first.subject.unwrap().id, again.subject.unwrap().id);
}

#[sqlx::test(migrations = "./migrations")]
async fn new_github_user_never_takes_an_existing_handle(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let original = login(&svc, "dave", 1, "").await.subject.unwrap();
    // Another GitHub account now called "dave" (the first one renamed away).
    let impostor = login(&svc, "dave", 2, "").await.subject.unwrap();
    assert_ne!(original.id, impostor.id);
    assert_eq!(original.handle, "dave");
    assert!(
        impostor.handle.starts_with("dave-"),
        "got {}",
        impostor.handle
    );
    assert_ne!(impostor.handle, "dave");
}

// ---------------------------------------------------------------------------
// sessions

#[sqlx::test(migrations = "./migrations")]
async fn refresh_rotates_and_reuse_revokes_the_family(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let t1 = login(&svc, "erin", 5, "").await;

    let t2 = svc
        .refresh_session(Request::new(RefreshSessionRequest {
            refresh_token: t1.refresh_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(t2.access_token, t1.access_token);
    assert_ne!(t2.refresh_token, t1.refresh_token);
    assert!(introspect(&svc, &t2.access_token).await.is_some());
    assert!(
        introspect(&svc, &t1.access_token).await.is_some(),
        "the old access token lives until it expires"
    );

    // Replaying the spent refresh token is theft: the whole family dies.
    let replay = svc
        .refresh_session(Request::new(RefreshSessionRequest {
            refresh_token: t1.refresh_token,
        }))
        .await;
    assert_eq!(code_of(replay), Code::Unauthenticated);
    assert!(
        introspect(&svc, &t2.access_token).await.is_none(),
        "the rotated session is revoked too"
    );
    assert!(introspect(&svc, &t1.access_token).await.is_none());
    let dead = svc
        .refresh_session(Request::new(RefreshSessionRequest {
            refresh_token: t2.refresh_token,
        }))
        .await;
    assert_eq!(code_of(dead), Code::Unauthenticated);
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_refresh_is_rejected(pool: PgPool) {
    let (svc, _fake) = service(pool.clone()).await;
    let t = login(&svc, "frank", 6, "").await;
    sqlx::query("update sessions set refresh_expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let r = svc
        .refresh_session(Request::new(RefreshSessionRequest {
            refresh_token: t.refresh_token,
        }))
        .await;
    assert_eq!(code_of(r), Code::Unauthenticated);
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_access_token_is_inactive(pool: PgPool) {
    let (svc, _fake) = service(pool.clone()).await;
    let t = login(&svc, "grace", 8, "").await;
    sqlx::query("update sessions set access_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(introspect(&svc, &t.access_token).await.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_revokes_the_access_token(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let t = login(&svc, "heidi", 9, "").await;
    svc.revoke_session(Request::new(RevokeSessionRequest {
        access_token: t.access_token.clone(),
    }))
    .await
    .unwrap();
    assert!(introspect(&svc, &t.access_token).await.is_none());
    // idempotent
    svc.revoke_session(Request::new(RevokeSessionRequest {
        access_token: t.access_token,
    }))
    .await
    .unwrap();
    svc.revoke_session(Request::new(RevokeSessionRequest {
        access_token: "unknown".into(),
    }))
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// requesters

#[sqlx::test(migrations = "./migrations")]
async fn client_credentials_issue_requester_tokens(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let owner = login(&svc, "ivan", 11, "").await.subject.unwrap().id;

    let created = svc
        .create_requester(Request::new(CreateRequesterRequest {
            owner_subject_id: owner.clone(),
            name: " ShopCo ".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    let requester = created.requester.unwrap();
    assert_eq!(requester.name, "ShopCo");
    assert!(requester.client_id.starts_with("afx_"));
    assert_eq!(requester.owner_subject_id, owner);
    assert_eq!(created.client_secret.len(), 43);

    let issued = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: requester.client_id.clone(),
            client_secret: created.client_secret.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(issued.expires_in_seconds, 3600);
    match introspect(&svc, &issued.access_token).await {
        Some(Principal::RequesterId(id)) => assert_eq!(id, requester.id),
        other => panic!("expected requester principal, got {other:?}"),
    }

    let wrong = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: requester.client_id.clone(),
            client_secret: "wrong".into(),
        }))
        .await;
    assert_eq!(code_of(wrong), Code::Unauthenticated);
    let unknown = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: "afx_nope".into(),
            client_secret: created.client_secret,
        }))
        .await;
    assert_eq!(code_of(unknown), Code::Unauthenticated);

    let fetched = svc
        .get_requester(Request::new(GetRequesterRequest {
            requester_id: requester.id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.client_id, requester.client_id);
    let missing = svc
        .get_requester(Request::new(GetRequesterRequest {
            requester_id: uuid::Uuid::now_v7().to_string(),
        }))
        .await;
    assert_eq!(code_of(missing), Code::NotFound);
}

#[sqlx::test(migrations = "./migrations")]
async fn rotation_is_owner_only_and_kills_old_secret_and_tokens(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let owner = login(&svc, "judy", 12, "").await.subject.unwrap().id;
    let other = login(&svc, "mallory", 13, "").await.subject.unwrap().id;

    let created = svc
        .create_requester(Request::new(CreateRequesterRequest {
            owner_subject_id: owner.clone(),
            name: "TalentHub".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    let requester = created.requester.unwrap();
    let old_token = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: requester.client_id.clone(),
            client_secret: created.client_secret.clone(),
        }))
        .await
        .unwrap()
        .into_inner()
        .access_token;

    let foreign = svc
        .rotate_requester_secret(Request::new(RotateRequesterSecretRequest {
            owner_subject_id: other,
            requester_id: requester.id.clone(),
        }))
        .await;
    assert_eq!(
        code_of(foreign),
        Code::NotFound,
        "a non-owner cannot rotate"
    );
    assert!(
        introspect(&svc, &old_token).await.is_some(),
        "nothing changed"
    );

    let rotated = svc
        .rotate_requester_secret(Request::new(RotateRequesterSecretRequest {
            owner_subject_id: owner,
            requester_id: requester.id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(rotated.client_secret, created.client_secret);
    assert!(
        introspect(&svc, &old_token).await.is_none(),
        "tokens die with the old secret"
    );

    let old = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: requester.client_id.clone(),
            client_secret: created.client_secret,
        }))
        .await;
    assert_eq!(code_of(old), Code::Unauthenticated);
    let new = svc
        .issue_client_token(Request::new(IssueClientTokenRequest {
            client_id: requester.client_id.clone(),
            client_secret: rotated.client_secret,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(introspect(&svc, &new.access_token).await.is_some());

    let fetched = svc
        .get_requester(Request::new(GetRequesterRequest {
            requester_id: requester.id,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        fetched.client_id, requester.client_id,
        "client_id survives rotation"
    );
    assert!(fetched.rotated_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn listing_is_a_directory_or_owner_scoped(pool: PgPool) {
    let (svc, _fake) = service(pool).await;
    let a = login(&svc, "kim", 20, "").await.subject.unwrap().id;
    let b = login(&svc, "leo", 21, "").await.subject.unwrap().id;
    for (owner, name) in [(&a, "A1"), (&a, "A2"), (&b, "B1")] {
        svc.create_requester(Request::new(CreateRequesterRequest {
            owner_subject_id: owner.clone(),
            name: name.into(),
        }))
        .await
        .unwrap();
    }
    let all = svc
        .list_requesters(Request::new(ListRequestersRequest {
            owner_subject_id: None,
        }))
        .await
        .unwrap()
        .into_inner()
        .requesters;
    assert_eq!(all.len(), 3);
    let mine = svc
        .list_requesters(Request::new(ListRequestersRequest {
            owner_subject_id: Some(a),
        }))
        .await
        .unwrap()
        .into_inner()
        .requesters;
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().all(|r| r.name.starts_with('A')));

    let bad = svc
        .create_requester(Request::new(CreateRequesterRequest {
            owner_subject_id: b,
            name: "   ".into(),
        }))
        .await;
    assert_eq!(code_of(bad), Code::InvalidArgument);
}

#[sqlx::test(migrations = "./migrations")]
async fn cleanup_deletes_only_expired_rows(pool: PgPool) {
    let (svc, _fake) = service(pool.clone()).await;
    let t = login(&svc, "mia", 30, "").await;
    svc.begin_github_login(Request::new(BeginGithubLoginRequest::default()))
        .await
        .unwrap();
    sqlx::query("update oauth_states set expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    let (states, tokens, sessions) = repo::delete_expired(&pool).await.unwrap();
    assert_eq!((states, tokens, sessions), (1, 0, 0));
    assert!(
        introspect(&svc, &t.access_token).await.is_some(),
        "live sessions survive cleanup"
    );
}
