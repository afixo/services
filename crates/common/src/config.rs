//! Environment-variable configuration. Services are configured exclusively
//! through the environment (12-factor); Kubernetes injects ConfigMap/Secret
//! values, local dev uses `.env` via the Makefile.

use std::{env, fmt::Display, str::FromStr};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("environment variable {name} has an invalid value: {reason}")]
    Invalid { name: &'static str, reason: String },
}

/// A required variable; absence is a startup failure, never a silent default.
pub fn required(name: &'static str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(ConfigError::Missing(name)),
    }
}

/// An optional variable with a default.
pub fn or(name: &'static str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// An optional variable, `None` when unset or blank.
pub fn optional(name: &'static str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Parse a variable into any `FromStr` type, falling back to `default` when unset.
pub fn parse_or<T>(name: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: Display,
{
    match optional(name) {
        None => Ok(default),
        Some(raw) => raw.parse::<T>().map_err(|e| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        }),
    }
}

/// Comma-separated list → trimmed, non-empty items.
pub fn list(name: &'static str) -> Vec<String> {
    optional(name)
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The address a gRPC service listens on. Defaults to all interfaces on 50051,
/// which is what the Kubernetes manifests expect.
pub fn grpc_addr() -> Result<std::net::SocketAddr, ConfigError> {
    parse_or(
        "GRPC_ADDR",
        "0.0.0.0:50051".parse().expect("valid default addr"),
    )
}
