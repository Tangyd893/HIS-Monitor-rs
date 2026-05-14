/// monitor-api 入口
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    monitor_api::run("0.0.0.0", 9100).await
}
