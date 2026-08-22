//! Route tables. Each listener has its own router; both share the handlers'
//! state and the same middleware stack.

use axum::{Json, Router};
use serde::Serialize;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod console;
pub mod machine;

#[derive(Debug, Serialize)]
pub struct Health {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: "gateway",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Request-id in, request-id out, one span per request.
pub fn with_common_layers(router: Router) -> Router {
    router
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
