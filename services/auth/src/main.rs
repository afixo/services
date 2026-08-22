//! auth-service entrypoint. See CLAUDE.md in this directory.

use afixo_common::{config, db, grpc, telemetry};
use afixo_proto::auth_service_server::AuthServiceServer;
use tonic::transport::Server;

mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("auth");

    let addr = config::grpc_addr()?;
    let pool = db::connect().await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let identity = grpc::channel("IDENTITY_URL")?;
    let service = AuthServiceServer::new(server::AuthSvc::new(pool, identity));
    let (_reporter, health) = grpc::health::<AuthServiceServer<server::AuthSvc>>().await;

    tracing::info!(%addr, "listening");
    Server::builder()
        .add_service(health)
        .add_service(service)
        .serve_with_shutdown(addr, grpc::shutdown_signal())
        .await?;
    Ok(())
}
