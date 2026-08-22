//! tonic helpers: health reporting, graceful shutdown, lazy client channels,
//! and the one place that maps "something unexpected happened" to a
//! client-safe `Status`.

use tonic::{Status, transport::Channel};
use tonic_health::{
    pb::health_server::{Health, HealthServer},
    server::{HealthReporter, health_reporter},
};

use crate::config::{self, ConfigError};

/// Resolve when SIGTERM (Kubernetes) or Ctrl-C (local) arrives.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl-c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

/// A health service pre-marked SERVING for `S`. The empty service name — what
/// the Kubernetes gRPC probe asks for — is SERVING by default.
pub async fn health<S: tonic::server::NamedService>() -> (HealthReporter, HealthServer<impl Health>)
{
    let (reporter, service) = health_reporter();
    reporter.set_serving::<S>().await;
    (reporter, service)
}

/// A lazily-connected channel to another service, from an env var such as
/// `AUTH_URL=http://auth:50051`. Lazy so boot order between services does not
/// matter; the first RPC connects (and fails fast with UNAVAILABLE if it can't).
pub fn channel(env_name: &'static str) -> Result<Channel, ConfigError> {
    let url = config::required(env_name)?;
    Channel::from_shared(url)
        .map(|endpoint| endpoint.connect_lazy())
        .map_err(|e| ConfigError::Invalid {
            name: env_name,
            reason: e.to_string(),
        })
}

/// Log the real error server-side, return an opaque INTERNAL to the caller.
pub fn internal(context: &'static str, err: impl std::fmt::Display) -> Status {
    tracing::error!(context, error = %err, "internal error");
    Status::internal(context)
}

/// Map a sqlx error: not-found stays semantic, unique violations become
/// ALREADY_EXISTS, everything else is internal.
pub fn db_status(context: &'static str, err: sqlx::Error) -> Status {
    match &err {
        sqlx::Error::RowNotFound => Status::not_found(context),
        sqlx::Error::Database(db) if db.is_unique_violation() => Status::already_exists(context),
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
            Status::failed_precondition(context)
        }
        _ => internal(context, err),
    }
}
