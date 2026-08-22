//! Machine listener (:8081) — the product surface for requesters via
//! `api.afixo.io`. Four routes, nothing else; the subject surface is not
//! reachable from this socket at all.

use afixo_proto as pb;
use axum::{
    Form, Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, Method, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{
    auth::{ClientCredentials, Requester},
    dto,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    let origins: Vec<HeaderValue> = state
        .cors_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_mins(10));

    let api = Router::new()
        .route("/v1/health", get(super::health))
        .route("/v1/purposes", get(super::console::purposes))
        .route("/oauth/token", post(token))
        .route("/v1/disclose/{handle}", get(disclose))
        .layer(cors)
        .with_state(state);
    super::with_common_layers(api)
}

/// RFC 6749 §4.4 client-credentials grant. Credentials via HTTP Basic or the
/// form body; the response is the standard token JSON with `Cache-Control: no-store`.
async fn token(
    State(state): State<AppState>,
    parts: Parts,
    Form(form): Form<dto::TokenForm>,
) -> ApiResult<Response> {
    match form.grant_type.as_deref() {
        Some("client_credentials") => {}
        Some(_) => {
            return Err(ApiError::bad_request(
                "unsupported_grant_type",
                "only client_credentials is supported",
            ));
        }
        None => {
            return Err(ApiError::bad_request(
                "invalid_request",
                "grant_type is required",
            ));
        }
    }
    let creds = ClientCredentials::from_basic(&parts)
        .or_else(|| ClientCredentials::from_form(form.client_id, form.client_secret))
        .ok_or_else(|| {
            ApiError::unauthorized("invalid_client", "client authentication required")
                .with_www_authenticate("Basic")
        })?;

    let resp = state
        .auth
        .clone()
        .issue_client_token(pb::IssueClientTokenRequest {
            client_id: creds.client_id,
            client_secret: creds.client_secret,
        })
        .await
        .map_err(|s| match s.code() {
            tonic::Code::Unauthenticated | tonic::Code::NotFound => {
                ApiError::unauthorized("invalid_client", "client authentication failed")
                    .with_www_authenticate("Basic")
            }
            _ => ApiError::from(s),
        })?
        .into_inner();

    let body = dto::ClientToken {
        access_token: resp.access_token,
        token_type: "Bearer",
        expires_in: resp.expires_in_seconds,
    };
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(body),
    )
        .into_response())
}

/// The product endpoint. Deny — for any reason, including an unknown subject —
/// is a uniform 403 so a requester cannot enumerate handles.
async fn disclose(
    State(state): State<AppState>,
    requester: Requester,
    Path(handle): Path<String>,
    Query(q): Query<dto::DiscloseQuery>,
) -> ApiResult<Response> {
    let purpose = q.purpose.filter(|p| !p.is_empty()).ok_or_else(|| {
        ApiError::bad_request("invalid_request", "purpose query parameter is required")
    })?;

    let resp = state
        .disclosure
        .clone()
        .disclose(pb::DiscloseRequest {
            subject_handle: handle,
            requester_id: requester.id.to_string(),
            purpose,
        })
        .await
        .map_err(|s| match s.code() {
            tonic::Code::InvalidArgument => {
                ApiError::bad_request("invalid_purpose", s.message().to_owned())
            }
            _ => ApiError::from(s),
        })?
        .into_inner();

    let no_store = [(header::CACHE_CONTROL, "no-store")];
    if resp.allowed {
        let body = dto::DiscloseBody {
            decision: "allow",
            decision_id: resp.decision_id,
            persona: Some(resp.persona_label),
            fields: Some(resp.fields.into_iter().collect()),
            withheld: Some(resp.withheld_keys),
            reason: None,
        };
        Ok((StatusCode::OK, no_store, Json(body)).into_response())
    } else {
        let body = dto::DiscloseBody {
            decision: "deny",
            decision_id: resp.decision_id,
            persona: None,
            fields: None,
            withheld: None,
            reason: Some("no_matching_rule".to_owned()),
        };
        Ok((StatusCode::FORBIDDEN, no_store, Json(body)).into_response())
    }
}
