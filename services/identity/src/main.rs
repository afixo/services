//! identity-service entrypoint. See CLAUDE.md in this directory.

use afixo_common::{config, db, grpc, telemetry};
use afixo_identity::server;
use afixo_proto::identity_service_server::IdentityServiceServer;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("identity");

    let addr = config::grpc_addr()?;
    let pool = db::connect().await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let service = IdentityServiceServer::new(server::IdentitySvc::new(pool));
    let (_reporter, health) = grpc::health::<IdentityServiceServer<server::IdentitySvc>>().await;

    tracing::info!(%addr, "listening");
    Server::builder()
        .add_service(health)
        .add_service(service)
        .serve_with_shutdown(addr, grpc::shutdown_signal())
        .await?;
    Ok(())
}
