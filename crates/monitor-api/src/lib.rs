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
        .route("/api/v1/alerts", get(handler::list_alerts))
        .route("/api/v1/alerts/{id}", get(handler::get_alert))
        .route(
            "/api/v1/alerts/{id}/ack",
            axum::routing::post(handler::ack_alert),
        )
        .route(
            "/api/v1/silences",
            get(handler::list_silences).post(handler::create_silence),
        )
        .route(
            "/api/v1/silences/{id}",
            axum::routing::delete(handler::delete_silence),
        )
        .route(
            "/api/v1/rules",
            get(handler::list_rules).post(handler::create_rule),
        )
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("monitor-api listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
