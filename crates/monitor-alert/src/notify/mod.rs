//! 通知通道适配器
//!
//! 提供钉钉、企业微信、邮件、短信、电话等多通道通知能力。
//! 通过 NotifyChannel trait 抽象各通道，NotifyDispatcher 统一调度。

pub mod channels;

use crate::manager::AlertStateChange;
use crate::router::{AlertRouter, Channel};
use crate::silence::SilenceManager;
use monitor_core::model::alert::Alert;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info};

// ── NotifyChannel trait ──

/// 通知通道抽象接口
#[async_trait::async_trait]
pub trait NotifyChannel: Send + Sync {
    /// 通道类型标识
    fn channel_type(&self) -> Channel;

    /// 发送告警通知
    async fn send(&self, alert: &Alert, receivers: &[String]) -> anyhow::Result<()>;

    /// 通道名称
    fn name(&self) -> &str;
}

// ── 通知分发器 ──

/// 通知分发器
///
/// 负责消费 AlertManager 的状态变更事件，
/// 通过 AlertRouter 确定通知通道，通过 SilenceManager 过滤已静默告警，
/// 并行调用各通道发送通知。
pub struct NotifyDispatcher {
    /// 路由器
    router: AlertRouter,
    /// 静默管理器（共享引用，与 API handler 共享同一实例）
    silence: Arc<Mutex<SilenceManager>>,
    /// 通道注册表
    channels: Vec<Box<dyn NotifyChannel>>,
}

impl NotifyDispatcher {
    /// 创建分发器
    pub fn new(router: AlertRouter, silence: Arc<Mutex<SilenceManager>>) -> Self {
        Self {
            router,
            silence,
            channels: Vec::new(),
        }
    }

    /// 注册通知通道
    pub fn register(&mut self, channel: Box<dyn NotifyChannel>) {
        debug!("registered channel: {}", channel.name());
        self.channels.push(channel);
    }

    /// 发送单条告警（忽略静默检查，用于手动发送）
    pub async fn send_alert(&self, alert: &Alert) -> Vec<Channel> {
        let route = self.router.route(alert);
        if route.channels.is_empty() {
            debug!(alert_id = alert.id, "no channels routed");
            return Vec::new();
        }

        let mut sent = Vec::new();
        for channel in &self.channels {
            if route.channels.contains(&channel.channel_type()) {
                match channel.send(alert, &route.receivers).await {
                    Ok(()) => {
                        sent.push(channel.channel_type());
                        debug!(
                            alert_id = alert.id,
                            channel = channel.name(),
                            "notification sent"
                        );
                    }
                    Err(e) => {
                        error!(
                            alert_id = alert.id,
                            channel = channel.name(),
                            error = %e,
                            "notification failed"
                        );
                    }
                }
            }
        }

        sent
    }

    /// 处理状态变更事件
    ///
    /// - Fired: 检查静默 → 发送通知
    /// - Resolved: 发送恢复通知
    /// - Acked/Pending: 不触发通知
    pub async fn handle_event(&self, event: &AlertStateChange) {
        match event {
            AlertStateChange::Fired(alert) => {
                // 检查静默（锁定共享的 SilenceManager）
                let silence = self.silence.lock().await;
                if let Some(rule) = silence.is_silenced(alert) {
                    info!(
                        alert_id = alert.id,
                        silence_id = rule.id,
                        reason = rule.reason,
                        "alert silenced, skipping notification"
                    );
                    return;
                }
                drop(silence);

                info!(
                    alert_id = alert.id,
                    rule = alert.rule_name,
                    level = ?alert.level,
                    "dispatching alert notification"
                );
                self.send_alert(alert).await;
            }

            AlertStateChange::Resolved(alert) => {
                info!(
                    alert_id = alert.id,
                    rule = alert.rule_name,
                    "dispatching resolve notification"
                );
                self.send_alert(alert).await;
            }

            AlertStateChange::Acked(alert) => {
                debug!(alert_id = alert.id, "alert acked, no notification needed");
            }

            AlertStateChange::Pending(_) => {
                // PENDING 阶段不发送通知
            }
        }
    }

    /// 运行分发器主循环
    ///
    /// 从 mpsc channel 接收 AlertStateChange 事件并处理。
    pub async fn run(self, mut event_rx: mpsc::Receiver<AlertStateChange>) {
        info!("notify dispatcher started, channels: {}", self.channels.len());

        loop {
            match event_rx.recv().await {
                Some(event) => {
                    self.handle_event(&event).await;
                }
                None => {
                    info!("event channel closed, stopping dispatcher");
                    break;
                }
            }
        }
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::channels::ConsoleChannel;
    use super::*;
    use chrono::Utc;
    use monitor_core::model::alert::{Alert, AlertLevel, AlertStatus};
    use uuid::Uuid;

    fn make_alert(level: AlertLevel) -> Alert {
        Alert {
            id: Uuid::new_v4().to_string(),
            rule_name: "test".into(),
            level,
            status: AlertStatus::Firing,
            summary: "test alert".into(),
            description: "test description".into(),
            service_name: "gateway".into(),
            fired_at: Utc::now(),
            acked_at: None,
            resolved_at: None,
            silenced_until: None,
            current_value: Some(42.0),
            labels: Vec::new(),
            group_by: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_dispatcher_sends_to_console() {
        let router = AlertRouter::with_defaults();
        let silence = Arc::new(Mutex::new(SilenceManager::new()));
        let mut dispatcher = NotifyDispatcher::new(router, silence);
        dispatcher.register(Box::new(ConsoleChannel::new()));

        let alert = make_alert(AlertLevel::Warning);
        let sent = dispatcher.send_alert(&alert).await;

        // Console 通道注册为支持所有 Channel 类型
        assert!(!sent.is_empty());
    }

    #[tokio::test]
    async fn test_dispatcher_handles_fired_event() {
        let router = AlertRouter::with_defaults();
        let silence = Arc::new(Mutex::new(SilenceManager::new()));
        let mut dispatcher = NotifyDispatcher::new(router, silence);
        dispatcher.register(Box::new(ConsoleChannel::new()));

        let alert = make_alert(AlertLevel::Critical);
        let event = AlertStateChange::Fired(alert);
        dispatcher.handle_event(&event).await;
        // 不应 panic
    }
}
