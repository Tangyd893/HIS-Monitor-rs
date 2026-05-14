//! API 请求处理器
//!
//! - health: 存活检查
//! - metrics: Prometheus 指标端点
//! - alerts: 告警查询与确认
//! - silences: 静默窗口管理
//! - rules: 告警规则管理

use crate::server::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use monitor_alert::rule::{AlertRule, CompareOp};
use monitor_alert::silence::{SilenceLabelMatcher, SilenceRule};
use monitor_core::model::alert::{AlertLevel, AlertStatus};
use prometheus::{Encoder, TextEncoder};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

// ── 健康检查 ──

/// 存活检查
pub async fn health() -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// Prometheus 指标端点
///
/// 从共享 Registry 编码 Prometheus 文本格式指标。
pub async fn metrics(State(state): State<AppState>) -> String {
    let encoder = TextEncoder::new();
    let metric_families = state.metrics_registry.gather();
    let mut buffer = vec![];
    encoder
        .encode(&metric_families, &mut buffer)
        .unwrap_or_default();
    String::from_utf8(buffer).unwrap_or_default()
}

// ── 告警 API ──

/// 告警列表查询参数
#[derive(Debug, Deserialize)]
pub struct AlertQuery {
    pub status: Option<String>,
    pub level: Option<String>,
    pub service: Option<String>,
    pub limit: Option<usize>,
}

/// 获取活跃告警列表
pub async fn list_alerts(
    State(state): State<AppState>,
    Query(query): Query<AlertQuery>,
) -> Json<Value> {
    let manager = state.alert_manager.lock().await;
    let alerts = manager.active_alerts();

    // 解析过滤条件
    let status_filter: Option<AlertStatus> = query
        .status
        .as_deref()
        .and_then(parse_alert_status);
    let level_filter: Option<AlertLevel> = query
        .level
        .as_deref()
        .and_then(parse_alert_level);

    let filtered: Vec<&monitor_core::model::alert::Alert> = alerts
        .iter()
        .filter(|a| {
            if let Some(ref s) = status_filter {
                a.status != *s
            } else {
                true
            }
        })
        .filter(|a| {
            if let Some(ref l) = level_filter {
                a.level == *l
            } else {
                true
            }
        })
        .filter(|a| {
            if let Some(ref svc) = query.service {
                a.service_name.contains(svc.as_str())
            } else {
                true
            }
        })
        .collect();

    let limit = query.limit.unwrap_or(100);
    let total = filtered.len();
    let items: Vec<&monitor_core::model::alert::Alert> =
        filtered.into_iter().take(limit).collect();

    Json(json!({
        "alerts": items,
        "total": total
    }))
}

/// 获取单个告警详情
pub async fn get_alert(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let manager = state.alert_manager.lock().await;
    let all = manager.all_alerts();
    match all.iter().find(|a| a.id == id) {
        Some(alert) => Json(json!(alert)),
        None => Json(json!({ "error": "alert not found" })),
    }
}

/// 确认告警
#[derive(Debug, Deserialize)]
pub struct AckRequest {
    pub comment: Option<String>,
}

pub async fn ack_alert(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AckRequest>,
) -> Json<Value> {
    let mut manager = state.alert_manager.lock().await;
    match manager.ack_alert(&id).await {
        Some(alert) => {
            info!(alert_id = id, comment = ?body.comment, "alert acked via API");
            Json(json!({
                "status": "acked",
                "id": alert.id
            }))
        }
        None => Json(json!({
            "error": "alert not found or already resolved"
        })),
    }
}

// ── 静默窗口 API ──

/// 静默创建请求
#[derive(Debug, Deserialize)]
pub struct CreateSilenceRequest {
    pub starts_at: String,
    pub ends_at: String,
    pub reason: String,
    #[serde(default)]
    pub matchers: Vec<SilenceMatcherRequest>,
    pub min_severity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SilenceMatcherRequest {
    pub key: String,
    pub value: String,
}

/// 创建静默窗口
pub async fn create_silence(
    State(state): State<AppState>,
    Json(body): Json<CreateSilenceRequest>,
) -> (StatusCode, Json<Value>) {
    // 解析时间
    let starts_at = match chrono::DateTime::parse_from_rfc3339(&body.starts_at) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid starts_at: {}", e) })),
            );
        }
    };
    let ends_at = match chrono::DateTime::parse_from_rfc3339(&body.ends_at) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid ends_at: {}", e) })),
            );
        }
    };

    let matchers: Vec<SilenceLabelMatcher> = body
        .matchers
        .into_iter()
        .map(|m| SilenceLabelMatcher {
            key: m.key,
            value: m.value,
        })
        .collect();

    let min_severity = body
        .min_severity
        .as_deref()
        .and_then(parse_alert_level);

    let rule = SilenceRule {
        id: String::new(), // 由 SilenceManager 自动生成
        created_by: "api".into(),
        reason: body.reason,
        starts_at,
        ends_at,
        matchers,
        min_severity,
    };

    let mut mgr = state.silence_manager.lock().await;
    let id = mgr.add(rule);

    info!(id = id, "silence created via API");

    (StatusCode::CREATED, Json(json!({
        "id": id,
        "status": "created"
    })))
}

/// 获取静默列表
pub async fn list_silences(
    State(state): State<AppState>,
) -> Json<Value> {
    let mgr = state.silence_manager.lock().await;
    let rules = mgr.all_rules();
    Json(json!({
        "silences": rules
    }))
}

/// 删除静默窗口
pub async fn delete_silence(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let mut mgr = state.silence_manager.lock().await;
    match mgr.remove(&id) {
        Some(_) => (StatusCode::OK, Json(json!({ "status": "deleted" }))),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "silence not found" })),
        ),
    }
}

// ── 规则管理 API ──

/// 规则创建请求（将 JSON 直接反序列化为 AlertRule）
#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub name: String,
    pub metric_pattern: String,
    #[serde(default)]
    pub label_matchers: Vec<LabelMatcherRequest>,
    pub op: String,
    pub threshold: f64,
    #[serde(default = "default_duration")]
    pub duration_secs: u64,
    pub level: String,
    pub summary: String,
    pub description: String,
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    #[serde(default)]
    pub group_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LabelMatcherRequest {
    pub key: String,
    pub value: String,
}

fn default_duration() -> u64 {
    60
}

/// 解析比较运算符
fn parse_op(s: &str) -> Option<CompareOp> {
    match s {
        ">" | "gt" => Some(CompareOp::Gt),
        "<" | "lt" => Some(CompareOp::Lt),
        ">=" | "gte" => Some(CompareOp::Gte),
        "<=" | "lte" => Some(CompareOp::Lte),
        "==" | "eq" => Some(CompareOp::Eq),
        "!=" | "neq" => Some(CompareOp::Neq),
        _ => None,
    }
}

/// 规则列表
pub async fn list_rules(
    State(state): State<AppState>,
) -> Json<Value> {
    let manager = state.alert_manager.lock().await;
    let rules = manager.rule_engine.all_rules();
    Json(json!({
        "rules": rules,
        "total": rules.len()
    }))
}

/// 创建规则
pub async fn create_rule(
    State(state): State<AppState>,
    Json(body): Json<CreateRuleRequest>,
) -> (StatusCode, Json<Value>) {
    let op = match parse_op(&body.op) {
        Some(op) => op,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid op: {}", body.op) })),
            );
        }
    };

    let level = match parse_alert_level(&body.level) {
        Some(l) => l,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("invalid level: {}", body.level) })),
            );
        }
    };

    let rule = AlertRule {
        name: body.name.clone(),
        metric_pattern: body.metric_pattern,
        label_matchers: body
            .label_matchers
            .into_iter()
            .map(|m| monitor_alert::rule::LabelMatcher {
                key: m.key,
                value: m.value,
            })
            .collect(),
        op,
        threshold: body.threshold,
        duration_secs: body.duration_secs,
        level,
        summary: body.summary,
        description: body.description,
        labels: body.labels,
        group_by: body.group_by,
    };

    let mut manager = state.alert_manager.lock().await;
    manager.add_rule(rule);

    info!(name = body.name, "rule created via API");

    (StatusCode::CREATED, Json(json!({
        "name": body.name,
        "status": "created"
    })))
}

// ── 辅助函数 ──

fn parse_alert_level(s: &str) -> Option<AlertLevel> {
    match s.to_lowercase().as_str() {
        "info" => Some(AlertLevel::Info),
        "warning" | "warn" => Some(AlertLevel::Warning),
        "critical" | "crit" => Some(AlertLevel::Critical),
        "emergency" | "emerg" => Some(AlertLevel::Emergency),
        _ => None,
    }
}

fn parse_alert_status(s: &str) -> Option<AlertStatus> {
    match s.to_lowercase().as_str() {
        "pending" => Some(AlertStatus::Pending),
        "firing" => Some(AlertStatus::Firing),
        "acked" => Some(AlertStatus::Acked),
        "resolved" => Some(AlertStatus::Resolved),
        "silenced" => Some(AlertStatus::Silenced),
        _ => None,
    }
}
