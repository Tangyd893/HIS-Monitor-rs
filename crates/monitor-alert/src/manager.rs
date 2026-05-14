//! 告警生命周期管理器
//!
//! 维护活跃告警的状态机：PENDING → FIRING → ACKED → RESOLVED。
//! 接收规则引擎评估结果，跟踪持续时间，发送状态变更通知。

use crate::rule::{AlertRule, RuleEngine, TriggerResult};
use monitor_core::model::alert::{Alert, AlertLevel, AlertStatus};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info};
use uuid::Uuid;

// ── 常量 ──

/// 默认恢复超时（秒）：如果告警在 FIRING/ACKED 状态存在超时后
/// 不再收到新的触发事件，自动转为 RESOLVED
const DEFAULT_RESOLVE_TIMEOUT_SECS: i64 = 300;

// ── 内部活跃告警 ──

/// 活跃告警的内部追踪记录
#[derive(Debug, Clone)]
struct ActiveAlert {
    /// 告警唯一 ID
    alert_id: String,
    /// 规则名称
    rule_name: String,
    /// 告警级别
    level: AlertLevel,
    /// 当前状态
    status: AlertStatus,
    /// 首次触发时间
    first_seen: DateTime<Utc>,
    /// 最后触发时间（用于判断恢复）
    last_seen: DateTime<Utc>,
    /// 规则要求的持续时间（秒），从 PENDING 变为 FIRING 所需
    duration_secs: u64,
    /// 摘要
    summary: String,
    /// 描述
    description: String,
    /// 标签
    labels: Vec<(String, String)>,
    /// 分组键
    group_by: Vec<String>,
    /// 服务名称
    service_name: String,
    /// 最新指标值
    current_value: f64,
    /// 恢复超时（秒）
    resolve_timeout_secs: i64,
}

impl ActiveAlert {
    /// 从 TriggerResult 创建新的活跃告警
    fn new(result: &TriggerResult, duration_secs: u64) -> Self {
        let service_name = result
            .labels
            .iter()
            .find(|(k, _)| k == "service")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| result.rule_name.clone());

        Self {
            alert_id: Uuid::new_v4().to_string(),
            rule_name: result.rule_name.clone(),
            level: result.level.clone(),
            status: AlertStatus::Pending,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            duration_secs,
            summary: result.summary.clone(),
            description: result.description.clone(),
            labels: result.labels.clone(),
            group_by: Vec::new(),
            current_value: result.current_value,
            resolve_timeout_secs: DEFAULT_RESOLVE_TIMEOUT_SECS,
            service_name,
        }
    }

    /// 刷新（收到新的触发事件）
    fn refresh(&mut self, result: &TriggerResult) {
        self.last_seen = Utc::now();
        self.current_value = result.current_value;
        self.summary = result.summary.clone();
        self.description = result.description.clone();
    }

    /// 检查是否应该从 PENDING 转为 FIRING
    fn should_fire(&self, now: DateTime<Utc>) -> bool {
        if self.status != AlertStatus::Pending {
            return false;
        }
        let elapsed = (now - self.first_seen).num_seconds() as u64;
        elapsed >= self.duration_secs
    }

    /// 检查是否应该恢复（超时未收到新触发）
    fn should_resolve(&self, now: DateTime<Utc>) -> bool {
        match self.status {
            AlertStatus::Pending
            | AlertStatus::Firing
            | AlertStatus::Acked
            | AlertStatus::Silenced => {
                let elapsed = (now - self.last_seen).num_seconds();
                elapsed > self.resolve_timeout_secs
            }
            AlertStatus::Resolved => false,
        }
    }

    /// 转为公开的 Alert 模型
    fn to_alert(&self) -> Alert {
        Alert {
            id: self.alert_id.clone(),
            rule_name: self.rule_name.clone(),
            level: self.level.clone(),
            status: self.status.clone(),
            summary: self.summary.clone(),
            description: self.description.clone(),
            service_name: self.service_name.clone(),
            fired_at: self.first_seen,
            acked_at: None,
            resolved_at: if matches!(self.status, AlertStatus::Resolved) {
                Some(Utc::now())
            } else {
                None
            },
            silenced_until: None,
            current_value: Some(self.current_value),
            labels: self.labels.clone(),
            group_by: self.group_by.clone(),
        }
    }
}

// ── 状态变更事件 ──

/// 告警状态变更事件
#[derive(Debug, Clone)]
pub enum AlertStateChange {
    /// 告警触发
    Fired(Alert),
    /// 告警恢复
    Resolved(Alert),
    /// 告警已确认
    Acked(Alert),
    /// PENDING 阶段（内部事件，通常不发送通知）
    Pending(Alert),
}

// ── 管理器 ──

/// 告警生命周期管理器
pub struct AlertManager {
    /// 规则引擎
    pub rule_engine: RuleEngine,
    /// 活跃告警映射 key: `rule_name::labels_fingerprint`
    active: HashMap<String, ActiveAlert>,
    /// 状态变更事件发送端（供外部 notify 模块消费）
    event_tx: mpsc::Sender<AlertStateChange>,
    /// 状态变更事件接收端
    event_rx: mpsc::Receiver<AlertStateChange>,
    /// 恢复检查间隔（秒）
    resolve_check_interval_secs: u64,
}

impl AlertManager {
    /// 创建新的告警管理器
    pub fn new(rule_engine: RuleEngine, event_buffer: usize) -> Self {
        let (event_tx, event_rx) = mpsc::channel(event_buffer);
        Self {
            rule_engine,
            active: HashMap::new(),
            event_tx,
            event_rx,
            resolve_check_interval_secs: 60,
        }
    }

    /// 设置恢复检查间隔
    pub fn with_resolve_interval(mut self, secs: u64) -> Self {
        self.resolve_check_interval_secs = secs;
        self
    }

    /// 获取事件接收端（用于外部消费）
    /// 注意：调用后原 `event_rx` 被消耗，后续通过 `event_sender_clone` 获取新的 sender。
    pub fn take_event_receiver(&mut self) -> mpsc::Receiver<AlertStateChange> {
        let (tx, rx) = mpsc::channel(self.event_rx.max_capacity());
        self.event_tx = tx;
        std::mem::replace(&mut self.event_rx, rx)
    }

    /// 获取事件发送端的克隆
    pub fn event_sender_clone(&self) -> mpsc::Sender<AlertStateChange> {
        self.event_tx.clone()
    }

    /// 添加规则
    pub fn add_rule(&mut self, rule: AlertRule) {
        self.rule_engine.add_rule(rule);
    }

    /// 获取规则数量
    pub fn rule_count(&self) -> usize {
        self.rule_engine.rule_count()
    }

    /// 获取活跃告警数量（不含已恢复）
    pub fn active_count(&self) -> usize {
        self.active
            .iter()
            .filter(|(_, a)| !matches!(a.status, AlertStatus::Resolved))
            .count()
    }

    /// 获取所有活跃告警（不含已恢复）
    pub fn active_alerts(&self) -> Vec<Alert> {
        self.active
            .values()
            .filter(|a| !matches!(a.status, AlertStatus::Resolved))
            .map(|a| a.to_alert())
            .collect()
    }

    /// 获取所有告警（含已恢复）
    pub fn all_alerts(&self) -> Vec<Alert> {
        self.active.values().map(|a| a.to_alert()).collect()
    }

    /// 处理一批 TriggerResult
    ///
    /// 返回状态变更事件列表。
    pub async fn process_results(&mut self, results: Vec<TriggerResult>) -> Vec<AlertStateChange> {
        let mut events = Vec::new();
        let now = Utc::now();

        for result in &results {
            let key = alert_key(&result.rule_name, &result.labels, &[]);

            if let Some(active) = self.active.get_mut(&key) {
                // 已有活跃告警，刷新最后触发时间
                active.refresh(result);

                // 检查是否从 PENDING → FIRING
                if active.should_fire(now) {
                    active.status = AlertStatus::Firing;
                    let alert = active.to_alert();
                    info!(
                        rule = active.rule_name,
                        alert_id = alert.id,
                        value = active.current_value,
                        "alert fired"
                    );
                    let evt = AlertStateChange::Fired(alert);
                    let _ = self.event_tx.send(evt.clone()).await;
                    events.push(evt);
                }
            } else {
                // 新告警，创建 PENDING 记录
                let mut active = ActiveAlert::new(result, result.duration_secs);
                debug!(
                    rule = active.rule_name,
                    alert_id = active.alert_id,
                    duration = active.duration_secs,
                    "alert pending"
                );

                // 如果持续时间为 0，直接触发
                if active.duration_secs == 0 {
                    active.status = AlertStatus::Firing;
                    let alert = active.to_alert();
                    info!(rule = active.rule_name, alert_id = alert.id, "alert fired immediately");
                    let evt = AlertStateChange::Fired(alert);
                    let _ = self.event_tx.send(evt.clone()).await;
                    events.push(evt);
                    self.active.insert(key, active);
                } else {
                    let evt = AlertStateChange::Pending(active.to_alert());
                    events.push(evt);
                    self.active.insert(key, active);
                }
            }
        }

        events
    }

    /// 定期检查恢复
    ///
    /// 应通过定时器周期调用（如每 60 秒）。
    pub async fn tick_resolve(&mut self) -> Vec<AlertStateChange> {
        let now = Utc::now();
        let mut events = Vec::new();
        let mut resolved_keys = Vec::new();

        for (key, active) in self.active.iter() {
            if active.should_resolve(now) {
                resolved_keys.push(key.clone());
            }
        }

        for key in &resolved_keys {
            if let Some(active) = self.active.get_mut(key) {
                active.status = AlertStatus::Resolved;
                let alert = active.to_alert();
                info!(rule = active.rule_name, alert_id = alert.id, "alert resolved");
                let evt = AlertStateChange::Resolved(alert);
                let _ = self.event_tx.send(evt.clone()).await;
                events.push(evt);
            }
        }

        // 清理超过 1 小时的已恢复告警
        self.active.retain(|_, a| {
            if matches!(a.status, AlertStatus::Resolved) {
                let age = (now - a.last_seen).num_seconds();
                age < 3600
            } else {
                true
            }
        });

        events
    }

    /// 确认告警（外部 API 调用）
    pub async fn ack_alert(&mut self, alert_id: &str) -> Option<Alert> {
        for (_, active) in self.active.iter_mut() {
            if active.alert_id == alert_id {
                if matches!(active.status, AlertStatus::Firing | AlertStatus::Pending) {
                    active.status = AlertStatus::Acked;
                    let mut alert = active.to_alert();
                    alert.acked_at = Some(Utc::now());
                    let evt = AlertStateChange::Acked(alert.clone());
                    let _ = self.event_tx.send(evt).await;
                    return Some(alert);
                }
                return Some(active.to_alert());
            }
        }
        None
    }

    /// 运行管理器主循环
    ///
    /// 在独立 task 中运行，接收 TriggerResult 流并定期执行恢复检查。
    pub async fn run(mut self, mut result_rx: mpsc::Receiver<Vec<TriggerResult>>) {
        let mut resolve_tick = tokio::time::interval(std::time::Duration::from_secs(
            self.resolve_check_interval_secs,
        ));

        info!("alert manager started");

        loop {
            tokio::select! {
                maybe_results = result_rx.recv() => {
                    match maybe_results {
                        Some(results) => {
                            let count = results.len();
                            let _ = self.process_results(results).await;
                            debug!(batch_size = count, "processed results");
                        }
                        None => {
                            info!("result channel closed, stopping alert manager");
                            break;
                        }
                    }
                }

                _ = resolve_tick.tick() => {
                    let events = self.tick_resolve().await;
                    if !events.is_empty() {
                        debug!(count = events.len(), "resolve tick processed");
                    }
                }
            }
        }
    }
}

// ── 辅助函数 ──

/// 生成告警去重键：rule_name + 排序后的标签指纹
fn alert_key(rule_name: &str, labels: &[(String, String)], group_by: &[String]) -> String {
    let mut filtered: Vec<_> = if group_by.is_empty() {
        labels.to_vec()
    } else {
        labels
            .iter()
            .filter(|(k, _)| group_by.contains(k))
            .cloned()
            .collect()
    };

    filtered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let fingerprint: String = filtered
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(",");

    format!("{}::{}", rule_name, fingerprint)
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::CompareOp;
    use monitor_core::model::metric::MetricSample;

    fn make_sample(name: &str, value: f64, labels: Vec<(&str, &str)>) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            value,
            timestamp: Utc::now(),
        }
    }

    fn make_rule(
        name: &str,
        metric: &str,
        op: CompareOp,
        threshold: f64,
        level: AlertLevel,
        duration: u64,
    ) -> AlertRule {
        AlertRule {
            name: name.into(),
            metric_pattern: metric.into(),
            label_matchers: vec![],
            op,
            threshold,
            duration_secs: duration,
            level,
            summary: format!("{name} is $value"),
            description: format!("{name} exceeded threshold"),
            labels: vec![],
            group_by: vec![],
        }
    }

    #[tokio::test]
    async fn test_immediate_fire_on_zero_duration() {
        let rule = make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning, 0);
        let engine = RuleEngine::new(vec![rule]);
        let mut manager = AlertManager::new(engine, 32);

        let sample = make_sample("cpu_usage", 0.85, vec![]);
        let results = manager.rule_engine.evaluate_sample(&sample);
        let events = manager.process_results(results).await;

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AlertStateChange::Fired(_)));
    }

    #[tokio::test]
    async fn test_pending_when_duration_required() {
        let rule = make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning, 300);
        let engine = RuleEngine::new(vec![rule]);
        let mut manager = AlertManager::new(engine, 32);

        let sample = make_sample("cpu_usage", 0.85, vec![]);
        let results = manager.rule_engine.evaluate_sample(&sample);
        let events = manager.process_results(results).await;

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AlertStateChange::Pending(_)));
        assert_eq!(manager.active_count(), 1);
    }

    #[tokio::test]
    async fn test_pending_to_firing_after_duration() {
        let rule = make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning, 1);
        let engine = RuleEngine::new(vec![rule]);
        let mut manager = AlertManager::new(engine, 32);

        // 第一次触发，duration=1s，进入 PENDING
        let sample = make_sample("cpu_usage", 0.85, vec![]);
        let results = manager.rule_engine.evaluate_sample(&sample);
        let events = manager.process_results(results).await;
        assert!(matches!(events[0], AlertStateChange::Pending(_)));

        // 等待 duration 过后再次触发
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let sample2 = make_sample("cpu_usage", 0.86, vec![]);
        let results2 = manager.rule_engine.evaluate_sample(&sample2);
        let events2 = manager.process_results(results2).await;

        assert!(events2.iter().any(|e| matches!(e, AlertStateChange::Fired(_))));
    }

    #[tokio::test]
    async fn test_resolve_on_timeout() {
        let rule = make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning, 0);
        let engine = RuleEngine::new(vec![rule]);
        let mut manager = AlertManager::new(engine, 32);

        let sample = make_sample("cpu_usage", 0.85, vec![]);
        let results = manager.rule_engine.evaluate_sample(&sample);
        let events = manager.process_results(results).await;
        assert!(matches!(events[0], AlertStateChange::Fired(_)));

        // 模拟超时
        for (_, active) in manager.active.iter_mut() {
            active.last_seen = Utc::now() - chrono::Duration::seconds(600);
        }

        let events = manager.tick_resolve().await;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AlertStateChange::Resolved(_)));
    }

    #[tokio::test]
    async fn test_ack_alert() {
        let rule = make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning, 0);
        let engine = RuleEngine::new(vec![rule]);
        let mut manager = AlertManager::new(engine, 32);

        let sample = make_sample("cpu_usage", 0.85, vec![]);
        let results = manager.rule_engine.evaluate_sample(&sample);
        manager.process_results(results).await;

        let alert_id = manager.active_alerts()[0].id.clone();
        let acked = manager.ack_alert(&alert_id).await;
        assert!(acked.is_some());
        assert!(matches!(acked.unwrap().status, AlertStatus::Acked));
    }

    #[test]
    fn test_alert_key_stable_order() {
        let key1 = alert_key("high_cpu", &[
            ("service".into(), "gateway".into()),
            ("env".into(), "prod".into()),
        ], &[]);

        let key2 = alert_key("high_cpu", &[
            ("env".into(), "prod".into()),
            ("service".into(), "gateway".into()),
        ], &[]);

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_alert_key_group_by() {
        let key1 = alert_key("high_cpu", &[
            ("service".into(), "gateway".into()),
            ("instance".into(), "pod-1".into()),
        ], &["service".to_string()]);

        let key2 = alert_key("high_cpu", &[
            ("service".into(), "gateway".into()),
            ("instance".into(), "pod-2".into()),
        ], &["service".to_string()]);

        assert_eq!(key1, key2);
    }
}
