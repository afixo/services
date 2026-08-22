//! Console listener (:8080) — the subject-facing surface behind Access.

use afixo_proto as pb;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post, put},
};

use crate::{
    auth::Subject,
    dto,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/v1/health", get(super::health))
        .route("/v1/purposes", get(purposes))
        // --- auth ---
        .route("/v1/auth/github/login", get(github_login))
        .route("/v1/auth/github/callback", get(github_callback))
        .route("/v1/auth/refresh", post(refresh))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/me", get(me))
        // --- personas ---
        .route("/v1/personas", get(list_personas).post(create_persona))
        .route("/v1/personas/{id}", get(get_persona).delete(delete_persona))
        .route("/v1/personas/{id}/fields/{key}", put(upsert_field).delete(delete_field))
        // --- requesters ---
        .route("/v1/requesters", get(list_requesters).post(create_requester))
        .route("/v1/requesters/{id}/secret", post(rotate_secret))
        // --- rules ---
        .route("/v1/rules", get(list_rules).post(create_rule))
        .route("/v1/rules/{id}", delete(delete_rule))
        // --- audit ---
        .route("/v1/audit", get(list_audit))
        .route("/v1/audit/verify", get(verify_audit))
        .with_state(state);
    super::with_common_layers(api)
}

pub async fn purposes(State(state): State<AppState>) -> ApiResult<Json<Vec<dto::Purpose>>> {
    let resp = state
        .policy
        .clone()
        .list_purposes(pb::ListPurposesRequest {})
        .await?
        .into_inner();
    Ok(Json(resp.purposes.into_iter().map(Into::into).collect()))
}

// ---------------------------------------------------------------------------
// auth

async fn github_login(
    State(state): State<AppState>,
    Query(q): Query<dto::LoginQuery>,
) -> ApiResult<Response> {
    let redirect_to = q.redirect_to.unwrap_or_default();
    let resp = state
        .auth
        .clone()
        .begin_github_login(pb::BeginGithubLoginRequest { redirect_to })
        .await?
        .into_inner();
    Ok(Redirect::to(&resp.authorize_url).into_response())
}

async fn github_callback(
    State(state): State<AppState>,
    Query(q): Query<dto::CallbackQuery>,
) -> ApiResult<Json<dto::Session>> {
    let tokens = state
        .auth
        .clone()
        .complete_github_login(pb::CompleteGithubLoginRequest {
            code: q.code,
            state: q.state,
        })
        .await
        .map_err(|s| match s.code() {
            tonic::Code::Unauthenticated | tonic::Code::NotFound | tonic::Code::InvalidArgument => {
                ApiError::unauthorized("login_failed", "GitHub login could not be completed")
            }
            _ => ApiError::from(s),
        })?
        .into_inner();
    Ok(Json(tokens.into()))
}

async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<dto::RefreshBody>,
) -> ApiResult<Json<dto::Session>> {
    let tokens = state
        .auth
        .clone()
        .refresh_session(pb::RefreshSessionRequest {
            refresh_token: body.refresh_token,
        })
        .await?
        .into_inner();
    Ok(Json(tokens.into()))
}

async fn logout(State(state): State<AppState>, subject: Subject) -> ApiResult<StatusCode> {
    state
        .auth
        .clone()
        .revoke_session(pb::RevokeSessionRequest {
            access_token: subject.token,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(state): State<AppState>, subject: Subject) -> ApiResult<Json<dto::Subject>> {
    let s = state
        .identity
        .clone()
        .get_subject(pb::GetSubjectRequest {
            subject_id: subject.id.to_string(),
        })
        .await?
        .into_inner();
    Ok(Json(s.into()))
}

// ---------------------------------------------------------------------------
// personas

async fn list_personas(
    State(state): State<AppState>,
    subject: Subject,
) -> ApiResult<Json<Vec<dto::Persona>>> {
    let resp = state
        .identity
        .clone()
        .list_personas(pb::ListPersonasRequest {
            subject_id: subject.id.to_string(),
        })
        .await?
        .into_inner();
    Ok(Json(resp.personas.into_iter().map(Into::into).collect()))
}

async fn create_persona(
    State(state): State<AppState>,
    subject: Subject,
    Json(body): Json<dto::CreatePersona>,
) -> ApiResult<(StatusCode, Json<dto::Persona>)> {
    let p = state
        .identity
        .clone()
        .create_persona(pb::CreatePersonaRequest {
            subject_id: subject.id.to_string(),
            label: body.label,
        })
        .await?
        .into_inner();
    Ok((StatusCode::CREATED, Json(p.into())))
}

async fn get_persona(
    State(state): State<AppState>,
    subject: Subject,
    Path(id): Path<String>,
) -> ApiResult<Json<dto::Persona>> {
    let p = state
        .identity
        .clone()
        .get_persona(pb::GetPersonaRequest {
            persona_id: id,
            subject_id: Some(subject.id.to_string()),
        })
        .await?
        .into_inner();
    Ok(Json(p.into()))
}

async fn delete_persona(
    State(state): State<AppState>,
    subject: Subject,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .identity
        .clone()
        .delete_persona(pb::DeletePersonaRequest {
            subject_id: subject.id.to_string(),
            persona_id: id,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn upsert_field(
    State(state): State<AppState>,
    subject: Subject,
    Path((id, key)): Path<(String, String)>,
    Json(body): Json<dto::UpsertField>,
) -> ApiResult<Json<dto::Persona>> {
    let p = state
        .identity
        .clone()
        .upsert_field(pb::UpsertFieldRequest {
            subject_id: subject.id.to_string(),
            persona_id: id,
            field: Some(pb::Field {
                key,
                value: body.value,
                sensitivity: body.sensitivity,
            }),
        })
        .await?
        .into_inner();
    Ok(Json(p.into()))
}

async fn delete_field(
    State(state): State<AppState>,
    subject: Subject,
    Path((id, key)): Path<(String, String)>,
) -> ApiResult<Json<dto::Persona>> {
    let p = state
        .identity
        .clone()
        .delete_field(pb::DeleteFieldRequest {
            subject_id: subject.id.to_string(),
            persona_id: id,
            key,
        })
        .await?
        .into_inner();
    Ok(Json(p.into()))
}

// ---------------------------------------------------------------------------
// requesters

async fn list_requesters(
    State(state): State<AppState>,
    subject: Subject,
    Query(q): Query<dto::RequestersQuery>,
) -> ApiResult<Json<Vec<dto::Requester>>> {
    let owner_subject_id = q.mine.then(|| subject.id.to_string());
    let resp = state
        .auth
        .clone()
        .list_requesters(pb::ListRequestersRequest { owner_subject_id })
        .await?
        .into_inner();
    Ok(Json(resp.requesters.into_iter().map(Into::into).collect()))
}

async fn create_requester(
    State(state): State<AppState>,
    subject: Subject,
    Json(body): Json<dto::CreateRequester>,
) -> ApiResult<(StatusCode, Json<dto::CreatedRequester>)> {
    let resp = state
        .auth
        .clone()
        .create_requester(pb::CreateRequesterRequest {
            owner_subject_id: subject.id.to_string(),
            name: body.name,
        })
        .await?
        .into_inner();
    let requester = resp.requester.ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "empty requester",
        )
    })?;
    Ok((
        StatusCode::CREATED,
        Json(dto::CreatedRequester {
            requester: requester.into(),
            client_secret: resp.client_secret,
        }),
    ))
}

async fn rotate_secret(
    State(state): State<AppState>,
    subject: Subject,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let resp = state
        .auth
        .clone()
        .rotate_requester_secret(pb::RotateRequesterSecretRequest {
            owner_subject_id: subject.id.to_string(),
            requester_id: id,
        })
        .await?
        .into_inner();
    Ok(Json(
        serde_json::json!({ "client_secret": resp.client_secret }),
    ))
}

// ---------------------------------------------------------------------------
// rules

async fn list_rules(
    State(state): State<AppState>,
    subject: Subject,
) -> ApiResult<Json<Vec<dto::Rule>>> {
    let resp = state
        .policy
        .clone()
        .list_rules(pb::ListRulesRequest {
            subject_id: subject.id.to_string(),
        })
        .await?
        .into_inner();
    Ok(Json(resp.rules.into_iter().map(Into::into).collect()))
}

async fn create_rule(
    State(state): State<AppState>,
    subject: Subject,
    Json(body): Json<dto::CreateRule>,
) -> ApiResult<(StatusCode, Json<dto::Rule>)> {
    let rule = state
        .policy
        .clone()
        .create_rule(pb::CreateRuleRequest {
            subject_id: subject.id.to_string(),
            requester_id: body.requester_id.filter(|s| !s.is_empty()),
            purpose: body.purpose.filter(|s| !s.is_empty()),
            persona_id: body.persona_id,
            max_sensitivity: body.max_sensitivity,
            allow_list: body.allow_keys.map(|keys| pb::AllowList { keys }),
            priority: body.priority,
        })
        .await?
        .into_inner();
    Ok((StatusCode::CREATED, Json(rule.into())))
}

async fn delete_rule(
    State(state): State<AppState>,
    subject: Subject,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .policy
        .clone()
        .delete_rule(pb::DeleteRuleRequest {
            subject_id: subject.id.to_string(),
            rule_id: id,
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// audit

async fn list_audit(
    State(state): State<AppState>,
    subject: Subject,
    Query(q): Query<dto::AuditQuery>,
) -> ApiResult<Json<dto::AuditPage>> {
    let resp = state
        .audit
        .clone()
        .list_decisions(pb::ListDecisionsRequest {
            subject_id: subject.id.to_string(),
            limit: q.limit.unwrap_or(50).clamp(1, 200),
            before_seq: q.before,
        })
        .await?
        .into_inner();
    Ok(Json(dto::AuditPage {
        decisions: resp.decisions.into_iter().map(Into::into).collect(),
        next_before: resp.next_before_seq,
    }))
}

async fn verify_audit(
    State(state): State<AppState>,
    _subject: Subject,
) -> ApiResult<Json<dto::ChainVerification>> {
    let resp = state
        .audit
        .clone()
        .verify_chain(pb::VerifyChainRequest {})
        .await?
        .into_inner();
    Ok(Json(dto::ChainVerification {
        ok: resp.ok,
        length: resp.length,
        broken_at_seq: resp.broken_at_seq,
        head_hash: hex::encode(&resp.head_hash),
    }))
}
