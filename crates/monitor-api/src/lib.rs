//! monitor-api — REST API 服务
//!
//! 基于 Axum 提供 REST API，向上层（Grafana / 运营管理后台）暴露监控数据查询接口。
//! 集成 AlertManager、SilenceManager、RuleEngine 和 AlertRouter。

use axum::{Router, routing::get};
use std::net::SocketAddr;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::info;

pub mod handler;
pub mod middleware;
pub mod server;

pub use server::AppState;

/// 启动 API 服务
pub async fn run(host: &str, port: u16) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // 构建应用状态和后台任务
    let (state, _resolve_task, _dispatcher_task) = server::build_state();

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
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(middleware::logger_middleware))
        .layer(axum::middleware::from_fn(middleware::tracing_middleware))
        .layer(middleware::RateLimitLayer::new(100, 60))
        .layer(middleware::cors_layer())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("monitor-api listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
