//! monitor-api — REST/gRPC API 服务
//!
//! 基于 Axum 提供 REST API，基于 Tonic 提供 gRPC API，
//! 向上层（Grafana / 运营管理后台）暴露监控数据查询接口。

use axum::{Router, routing::get};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

mod handler;
mod middleware;
mod server;

/// 启动 API 服务
pub async fn run(host: &str, port: u16) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let app = Router::new()
        .route("/health", get(handler::health))
        .route("/metrics", get(handler::metrics))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("monitor-api listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
