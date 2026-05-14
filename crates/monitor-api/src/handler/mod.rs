use axum::{Json, http::StatusCode};
use serde_json::{json, Value};

/// 存活检查
pub async fn health() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// Prometheus 指标端点
pub async fn metrics() -> String {
    // TODO: 返回实际的 Prometheus 格式指标
    "# HELP monitor_api_health Monitor API health status\n# TYPE monitor_api_health gauge\nmonitor_api_health 1\n".into()
}
