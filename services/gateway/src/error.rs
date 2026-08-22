//! The one error type every handler returns, and the gRPC → HTTP mapping.
//! Wire shape: `{"error":"<snake_code>","message":"<human>"}` (DESIGN §5).

use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tonic::{Code, Status};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// Set for 401s on the machine listener (RFC 6750 / 6749).
    pub www_authenticate: Option<&'static str>,
}

#[derive(Serialize)]
struct Body<'a> {
    error: &'a str,
    message: &'a str,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            www_authenticate: None,
        }
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn unauthorized(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message).with_www_authenticate("Bearer")
    }

    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub fn with_www_authenticate(mut self, scheme: &'static str) -> Self {
        self.www_authenticate = Some(scheme);
        self
    }
}

impl From<Status> for ApiError {
    fn from(s: Status) -> Self {
        let msg = s.message().to_owned();
        match s.code() {
            Code::InvalidArgument => Self::bad_request("invalid_request", msg),
            Code::NotFound => Self::new(StatusCode::NOT_FOUND, "not_found", msg),
            Code::AlreadyExists => Self::new(StatusCode::CONFLICT, "conflict", msg),
            Code::Unauthenticated => Self::unauthorized("invalid_token", msg),
            Code::PermissionDenied => Self::forbidden("forbidden", msg),
            Code::FailedPrecondition => Self::new(StatusCode::CONFLICT, "failed_precondition", msg),
            Code::ResourceExhausted => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", msg)
            }
            Code::Unimplemented => Self::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", msg),
            Code::Unavailable | Code::DeadlineExceeded => {
                tracing::warn!(code = ?s.code(), message = %msg, "upstream unavailable");
                Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "upstream_unavailable",
                    "a backend service is unavailable",
                )
            }
            other => {
                tracing::error!(code = ?other, message = %msg, "upstream error");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(Body {
            error: self.code,
            message: &self.message,
        });
        let mut response = (self.status, body).into_response();
        if let Some(scheme) = self.www_authenticate
            && let Ok(v) = header::HeaderValue::from_str(scheme)
        {
            response.headers_mut().insert(header::WWW_AUTHENTICATE, v);
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
