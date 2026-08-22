//! audit-service entrypoint. See CLAUDE.md in this directory.

use afixo_audit::server;
use afixo_common::{config, db, grpc, telemetry};
use afixo_proto::audit_service_server::AuditServiceServer;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("audit");

    let addr = config::grpc_addr()?;
    let pool = db::connect().await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let service = AuditServiceServer::new(server::AuditSvc::new(pool));
    let (_reporter, health) = grpc::health::<AuditServiceServer<server::AuditSvc>>().await;

    tracing::info!(%addr, "listening");
    Server::builder()
        .add_service(health)
        .add_service(service)
        .serve_with_shutdown(addr, grpc::shutdown_signal())
        .await?;
    Ok(())
}
