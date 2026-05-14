/// monitor-probe 入口
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base_url = std::env::var("GATEWAY_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let interval = std::env::var("PROBE_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    monitor_probe::run(&base_url, interval).await
}
