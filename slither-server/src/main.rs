#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mcp_mode = args.iter().any(|a| a == "--mcp");

    let config = slither_server::ServerConfig {
        host: std::env::var("SLITHER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: std::env::var("SLITHER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3001),
        api_key: std::env::var("SLITHER_API_KEY").ok(),
    };

    slither_server::run(mcp_mode, config).await
}
