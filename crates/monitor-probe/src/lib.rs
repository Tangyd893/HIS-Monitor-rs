//! monitor-probe — 业务链路探活代理
//!
//! 独立二进制，模拟真实用户行为，以黑盒视角验证 HIS-Go 核心业务链路端到端健康状态。

use reqwest::Client;
use std::time::Duration;
use tracing::{info, error};

mod engine;
mod scenario;

/// 启动探活代理
pub async fn run(interval_secs: u64) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    info!("monitor-probe started, interval={}s", interval_secs);

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = probe_once(&client).await {
            error!("probe failed: {}", e);
        }
    }
}

async fn probe_once(client: &Client) -> anyhow::Result<()> {
    // TODO: 按场景编排执行探活
    let resp = client.get("http://localhost:8080/health").send().await?;
    info!("gateway health: status={}", resp.status());
    Ok(())
}
