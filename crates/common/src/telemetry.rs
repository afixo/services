//! `tracing` setup. `RUST_LOG` controls the filter (default `info`),
//! `LOG_FORMAT=json` switches to one-JSON-object-per-line for the cluster.
//!
//! Rule for every service: never log a token, secret, cookie or persona field
//! value. Log ids, outcomes and durations.

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init(service: &'static str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = std::env::var("LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"));

    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry
            .with(
                fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_target(true),
            )
            .init();
    } else {
        registry
            .with(fmt::layer().compact().with_target(true))
            .init();
    }
    tracing::info!(service, version = env!("CARGO_PKG_VERSION"), "starting");
}
