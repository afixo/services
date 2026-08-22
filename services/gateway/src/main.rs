//! gateway entrypoint: two HTTP listeners, one process. See CLAUDE.md.
//!
//! * console listener (`CONSOLE_ADDR`, default :8080) — the full subject
//!   surface. Only reachable through `origin.afixo.io`, which Cloudflare Access
//!   restricts to the api Worker's service token.
//! * machine listener (`MACHINE_ADDR`, default :8081) — the product surface for
//!   requesters via `api.afixo.io`: token endpoint + disclose.
//!
//! cloudflared routes each hostname to its port, so the split is enforced by
//! which socket a request arrived on — nothing parses `Host`.

use std::net::SocketAddr;

use afixo_common::{config, grpc, telemetry};
use tokio::net::TcpListener;

mod auth;
mod dto;
mod error;
mod routes;
mod state;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("gateway");

    let console_addr: SocketAddr = config::parse_or("CONSOLE_ADDR", "0.0.0.0:8080".parse()?)?;
    let machine_addr: SocketAddr = config::parse_or("MACHINE_ADDR", "0.0.0.0:8081".parse()?)?;

    let state = state::AppState::from_env()?;
    let console = routes::console::router(state.clone());
    let machine = routes::machine::router(state);

    let console_listener = TcpListener::bind(console_addr).await?;
    let machine_listener = TcpListener::bind(machine_addr).await?;
    tracing::info!(%console_addr, %machine_addr, "listening");

    let (a, b) = tokio::join!(
        axum::serve(console_listener, console).with_graceful_shutdown(grpc::shutdown_signal()),
        axum::serve(machine_listener, machine).with_graceful_shutdown(grpc::shutdown_signal()),
    );
    a?;
    b?;
    Ok(())
}
