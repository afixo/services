//! Postgres pool construction. Every service owns exactly one database; the URL
//! arrives as `DATABASE_URL`. Migrations are embedded at compile time with
//! `sqlx::migrate!` and applied by the service at boot (single-replica services;
//! sqlx takes an advisory lock so concurrent boots are safe).

use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::config;

pub async fn connect() -> anyhow::Result<PgPool> {
    let url = config::required("DATABASE_URL")?;
    let max = config::parse_or::<u32>("DB_MAX_CONNECTIONS", 8)?;
    let pool = PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await?;
    tracing::info!(max_connections = max, "database pool ready");
    Ok(pool)
}
