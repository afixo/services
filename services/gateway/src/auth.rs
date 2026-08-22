//! Authentication extractors. Every bearer token — subject access token or
//! requester token — is resolved by auth-service's `IntrospectToken`; the
//! gateway never interprets a token itself.

use afixo_proto::{IntrospectTokenRequest, introspect_token_response::Principal};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

fn bearer(parts: &Parts) -> Result<String, ApiError> {
    let value = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("invalid_token", "missing bearer token"))?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::unauthorized("invalid_token", "malformed Authorization header"))?;
    Ok(token.to_owned())
}

async fn introspect(state: &AppState, token: &str) -> Result<Principal, ApiError> {
    let resp = state
        .auth
        .clone()
        .introspect_token(IntrospectTokenRequest {
            token: token.to_owned(),
        })
        .await?
        .into_inner();
    match (resp.active, resp.principal) {
        (true, Some(p)) => Ok(p),
        _ => Err(ApiError::unauthorized(
            "invalid_token",
            "token is invalid or expired",
        )),
    }
}

/// An authenticated subject (console listener). Carries the raw token so
/// logout can revoke it.
#[derive(Debug, Clone)]
pub struct Subject {
    pub id: Uuid,
    pub token: String,
}

impl FromRequestParts<AppState> for Subject {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts)?;
        match introspect(state, &token).await? {
            Principal::SubjectId(id) => {
                let id = Uuid::parse_str(&id).map_err(|_| {
                    ApiError::new(
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "internal",
                        "bad principal",
                    )
                })?;
                Ok(Self { id, token })
            }
            Principal::RequesterId(_) => Err(ApiError::forbidden(
                "wrong_principal",
                "a requester token cannot use the console API",
            )),
        }
    }
}

/// An authenticated requester (machine listener).
#[derive(Debug, Clone)]
pub struct Requester {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for Requester {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts)?;
        match introspect(state, &token).await? {
            Principal::RequesterId(id) => {
                let id = Uuid::parse_str(&id).map_err(|_| {
                    ApiError::new(
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "internal",
                        "bad principal",
                    )
                })?;
                Ok(Self { id })
            }
            Principal::SubjectId(_) => Err(ApiError::forbidden(
                "wrong_principal",
                "a subject session cannot call the machine API",
            )),
        }
    }
}

/// OAuth 2.0 client credentials, from HTTP Basic (preferred, RFC 6749 §2.3.1)
/// or from the form body.
#[derive(Debug, Clone)]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

impl ClientCredentials {
    pub fn from_basic(parts: &Parts) -> Option<Self> {
        let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let encoded = value.strip_prefix("Basic ")?;
        let decoded = STANDARD.decode(encoded.trim()).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let (id, secret) = decoded.split_once(':')?;
        (!id.is_empty() && !secret.is_empty()).then(|| Self {
            client_id: id.to_owned(),
            client_secret: secret.to_owned(),
        })
    }

    pub fn from_form(client_id: Option<String>, client_secret: Option<String>) -> Option<Self> {
        match (client_id, client_secret) {
            (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => Some(Self {
                client_id: id,
                client_secret: secret,
            }),
            _ => None,
        }
    }
}
