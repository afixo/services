//! policy-service entrypoint. See CLAUDE.md in this directory.

use afixo_common::{config, db, grpc, telemetry};
use afixo_proto::policy_service_server::PolicyServiceServer;
use tonic::transport::Server;

mod repo;
mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("policy");

    let addr = config::grpc_addr()?;
    let pool = db::connect().await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let identity = grpc::channel("IDENTITY_URL")?;
    let auth = grpc::channel("AUTH_URL")?;
    let service = PolicyServiceServer::new(server::PolicySvc::new(pool, identity, auth));
    let (_reporter, health) = grpc::health::<PolicyServiceServer<server::PolicySvc>>().await;

    tracing::info!(%addr, "listening");
    Server::builder()
        .add_service(health)
        .add_service(service)
        .serve_with_shutdown(addr, grpc::shutdown_signal())
        .await?;
    Ok(())
}
