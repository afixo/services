//! disclosure-service entrypoint. See CLAUDE.md in this directory.

use afixo_common::{config, grpc, telemetry};
use afixo_proto::disclosure_service_server::DisclosureServiceServer;
use tonic::transport::Server;

mod server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("disclosure");

    let addr = config::grpc_addr()?;
    let identity = grpc::channel("IDENTITY_URL")?;
    let policy = grpc::channel("POLICY_URL")?;
    let audit = grpc::channel("AUDIT_URL")?;

    let service = DisclosureServiceServer::new(server::DisclosureSvc::new(identity, policy, audit));
    let (_reporter, health) =
        grpc::health::<DisclosureServiceServer<server::DisclosureSvc>>().await;

    tracing::info!(%addr, "listening");
    Server::builder()
        .add_service(health)
        .add_service(service)
        .serve_with_shutdown(addr, grpc::shutdown_signal())
        .await?;
    Ok(())
}
