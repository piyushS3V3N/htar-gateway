use htar_proxy::{GatewayConfig, GatewayServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mut config = GatewayConfig::default();
    config.server.port = 8080;

    println!("Starting HTAR API Gateway example on http://127.0.0.1:8080 ...");
    let server = GatewayServer::new(config);
    server.run().await?;

    Ok(())
}
