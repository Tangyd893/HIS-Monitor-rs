/// monitor-probe 入口
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    monitor_probe::run(30).await
}
