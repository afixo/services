//! Hourly deletion of expired states, tokens and long-dead sessions. Hygiene,
//! not security: every lookup already checks expiry in its predicate.

use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinHandle;

use crate::repo;

pub fn spawn(pool: PgPool) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_hours(1));
        loop {
            tick.tick().await;
            match repo::delete_expired(&pool).await {
                Ok((states, tokens, sessions)) => {
                    tracing::info!(states, tokens, sessions, "expired rows deleted");
                }
                Err(e) => tracing::warn!(error = %e, "cleanup failed; will retry next hour"),
            }
        }
    })
}
