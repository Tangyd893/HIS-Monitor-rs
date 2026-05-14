//! API 服务器状态管理与组件组装
//!
//! 负责创建 AlertManager、SilenceManager、AlertRouter 等组件，
//! 将它们包装为 AppState 并注入到 Axum Router。

use monitor_alert::manager::AlertManager;
use monitor_alert::notify::NotifyDispatcher;
use monitor_alert::notify::channels::ConsoleChannel;
use monitor_alert::router::{AlertRouter, RouterConfig};
use monitor_alert::rule::{AlertRule, CompareOp, RuleEngine};
use monitor_alert::silence::SilenceManager;
use monitor_core::model::alert::AlertLevel;
use prometheus::Registry;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 共享应用状态，注入到所有 API handler
#[derive(Clone)]
pub struct AppState {
    /// 告警管理器（含规则引擎）
    pub alert_manager: Arc<Mutex<AlertManager>>,
    /// 静默窗口管理器
    pub silence_manager: Arc<Mutex<SilenceManager>>,
    /// 告警路由器（只读）
    pub router: Arc<AlertRouter>,
    /// Prometheus 指标注册表
    pub metrics_registry: Registry,
}

/// 组装默认规则集
fn default_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "high_cpu".into(),
            metric_pattern: "cpu_usage".into(),
            label_matchers: vec![],
            op: CompareOp::Gt,
            threshold: 0.85,
            duration_secs: 60,
            level: AlertLevel::Warning,
            summary: "CPU 使用率过高: $value".into(),
            description: "服务器 CPU 使用率 $value 超过阈值 0.85，持续 60 秒".into(),
            labels: vec![("component".into(), "cpu".into())],
            group_by: vec!["host".into()],
        },
        AlertRule {
            name: "high_memory".into(),
            metric_pattern: "mem_usage".into(),
            label_matchers: vec![],
            op: CompareOp::Gt,
            threshold: 0.90,
            duration_secs: 120,
            level: AlertLevel::Critical,
            summary: "内存使用率过高: $value".into(),
            description: "服务器内存使用率 $value 超过阈值 0.90，持续 120 秒".into(),
            labels: vec![("component".into(), "memory".into())],
            group_by: vec!["host".into()],
        },
        AlertRule {
            name: "http_error_rate".into(),
            metric_pattern: "http_errors".into(),
            label_matchers: vec![],
            op: CompareOp::Gt,
            threshold: 100.0,
            duration_secs: 30,
            level: AlertLevel::Critical,
            summary: "HTTP 错误数过高: $value".into(),
            description: "最近 30 秒 HTTP 错误数 $value 超过阈值 100".into(),
            labels: vec![("component".into(), "http".into())],
            group_by: vec!["service".into()],
        },
        AlertRule {
            name: "disk_low".into(),
            metric_pattern: "disk_free_percent".into(),
            label_matchers: vec![],
            op: CompareOp::Lt,
            threshold: 10.0,
            duration_secs: 0,
            level: AlertLevel::Critical,
            summary: "磁盘剩余空间不足: $value%".into(),
            description: "磁盘剩余空间仅 $value%，低于阈值 10%".into(),
            labels: vec![("component".into(), "disk".into())],
            group_by: vec!["host".into(), "mount".into()],
        },
        AlertRule {
            name: "pg_connections_high".into(),
            metric_pattern: "pg_connections_active".into(),
            label_matchers: vec![],
            op: CompareOp::Gt,
            threshold: 80.0,
            duration_secs: 120,
            level: AlertLevel::Warning,
            summary: "PG 活跃连接数过高: $value".into(),
            description: "PostgreSQL 活跃连接数 $value 超过阈值 80".into(),
            labels: vec![("component".into(), "database".into())],
            group_by: vec![],
        },
        AlertRule {
            name: "redis_memory_high".into(),
            metric_pattern: "redis_memory_used_bytes".into(),
            label_matchers: vec![],
            op: CompareOp::Gt,
            threshold: 858993459.0, // 800MB
            duration_secs: 60,
            level: AlertLevel::Warning,
            summary: "Redis 内存使用过高: $value bytes".into(),
            description: "Redis 内存使用 $value 超过 800MB".into(),
            labels: vec![("component".into(), "cache".into())],
            group_by: vec![],
        },
    ]
}

/// 构建应用状态并启动后台任务
///
/// 返回 `AppState` 和两个后台任务 join handle：
/// - `resolve_task`: 定期检查告警恢复
/// - `dispatcher_task`: 消费告警事件并分发通知
pub fn build_state() -> (AppState, tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
    // 1. 创建规则引擎和默认规则
    let rule_engine = RuleEngine::new(default_rules());

    // 2. 创建告警管理器
    let mut alert_manager = AlertManager::new(rule_engine, 256);

    // 3. 取出事件接收器（给 notify dispatcher）
    let event_rx = alert_manager.take_event_receiver();

    // 4. 创建静默管理器
    let silence_manager = SilenceManager::new();

    // 5. 创建路由器
    let router = AlertRouter::new(RouterConfig::default());

    // 6. 创建 Prometheus 指标注册表并注册默认指标
    let metrics_registry = Registry::new();
    register_default_metrics(&metrics_registry);

    // 7. 包装为共享状态
    let alert_manager = Arc::new(Mutex::new(alert_manager));
    let silence_manager = Arc::new(Mutex::new(silence_manager));
    let router = Arc::new(router);

    // 8. 启动后台恢复检查任务
    let am_clone = alert_manager.clone();
    let resolve_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let mut manager = am_clone.lock().await;
            let _events = manager.tick_resolve().await;
        }
    });

    // 9. 启动通知分发器后台任务 — 共享 SilenceManager 进行静默检查
    let sm_clone = silence_manager.clone();
    let router_clone = router.clone();
    let dispatcher_task = tokio::spawn(async move {
        let mut dispatcher = NotifyDispatcher::new((*router_clone).clone(), sm_clone);
        dispatcher.register(Box::new(ConsoleChannel::new()));
        dispatcher.run(event_rx).await;
    });

    let state = AppState {
        alert_manager,
        silence_manager,
        router,
        metrics_registry,
    };

    (state, resolve_task, dispatcher_task)
}

/// 向 Prometheus Registry 注册默认指标
fn register_default_metrics(registry: &Registry) {
    use prometheus::{IntCounter, IntGauge, Opts};

    // API 请求总数
    let api_requests = IntCounter::with_opts(
        Opts::new("monitor_api_requests_total", "Total number of API requests")
            .namespace("monitor"),
    )
    .unwrap();
    registry.register(Box::new(api_requests)).unwrap();

    // 健康状态
    let health = IntGauge::with_opts(
        Opts::new("monitor_api_health", "API health status (1 = healthy)")
            .namespace("monitor"),
    )
    .unwrap();
    health.set(1);
    registry.register(Box::new(health)).unwrap();

    // 活跃告警数（占位，实际由 collector 填充）
    let active_alerts = IntGauge::with_opts(
        Opts::new(
            "monitor_active_alerts",
            "Number of currently active (firing) alerts",
        )
        .namespace("monitor"),
    )
    .unwrap();
    registry.register(Box::new(active_alerts)).unwrap();

    // 活跃静默规则数
    let active_silences = IntGauge::with_opts(
        Opts::new(
            "monitor_active_silences",
            "Number of currently active silence rules",
        )
        .namespace("monitor"),
    )
    .unwrap();
    registry.register(Box::new(active_silences)).unwrap();
}
