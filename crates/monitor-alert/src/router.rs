//! 告警路由与分组
//!
//! 根据告警级别和标签将告警路由到不同的通知通道。
//! 支持默认路由表配置，以及按标签覆盖的路由规则。

use monitor_core::model::alert::{Alert, AlertLevel};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

// ── 通知通道 ──

/// 通知通道类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Channel {
    /// 电话（P0 紧急场景）
    Phone,
    /// 短信
    Sms,
    /// 钉钉群机器人
    DingTalk,
    /// 企业微信群机器人
    WeCom,
    /// 邮件
    Email,
}

impl Channel {
    /// 通道的展示名称
    pub fn display_name(&self) -> &str {
        match self {
            Channel::Phone => "电话",
            Channel::Sms => "短信",
            Channel::DingTalk => "钉钉",
            Channel::WeCom => "企业微信",
            Channel::Email => "邮件",
        }
    }
}

// ── 路由配置 ──

/// 单个路由条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    /// 级别（None 表示所有级别）
    pub level: Option<AlertLevel>,
    /// 通道列表
    pub channels: Vec<Channel>,
    /// 接收者列表（如群 ID、邮箱列表、手机号）
    #[serde(default)]
    pub receivers: Vec<String>,
}

/// 标签覆盖路由：当告警匹配特定标签时，使用独立的路由规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelRoute {
    /// 标签键
    pub key: String,
    /// 标签值
    pub value: String,
    /// 覆盖的通道
    pub channels: Vec<Channel>,
    /// 覆盖的接收者
    #[serde(default)]
    pub receivers: Vec<String>,
}

/// 告警路由器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// 默认路由表（按级别）
    pub routes: Vec<RouteEntry>,
    /// 标签覆盖路由（优先级高于级别路由）
    #[serde(default)]
    pub label_routes: Vec<LabelRoute>,
    /// 告警分组配置：同一时间窗口内，相同分组键的告警合并
    #[serde(default = "default_group_window_secs")]
    pub group_window_secs: u64,
}

fn default_group_window_secs() -> u64 {
    300
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            routes: default_routes(),
            label_routes: Vec::new(),
            group_window_secs: 300,
        }
    }
}

/// 默认路由表：按 P0-P3 配置
fn default_routes() -> Vec<RouteEntry> {
    vec![
        RouteEntry {
            level: Some(AlertLevel::Emergency),
            channels: vec![Channel::Phone, Channel::WeCom, Channel::DingTalk],
            receivers: vec!["his-oncall-group".into()],
        },
        RouteEntry {
            level: Some(AlertLevel::Critical),
            channels: vec![Channel::WeCom, Channel::DingTalk, Channel::Email],
            receivers: vec!["his-oncall-group".into(), "devops@hospital.com".into()],
        },
        RouteEntry {
            level: Some(AlertLevel::Warning),
            channels: vec![Channel::Email],
            receivers: vec!["devops@hospital.com".into()],
        },
        RouteEntry {
            level: Some(AlertLevel::Info),
            channels: vec![Channel::WeCom],
            receivers: vec!["his-silent-group".into()],
        },
    ]
}

// ── 路由结果 ──

/// 单条路由结果
#[derive(Debug, Clone)]
pub struct RouteResult {
    /// 告警 ID
    pub alert_id: String,
    /// 目标通道
    pub channels: Vec<Channel>,
    /// 接收者列表
    pub receivers: Vec<String>,
}

// ── 路由器 ──

/// 告警路由器
#[derive(Debug, Clone)]
pub struct AlertRouter {
    config: RouterConfig,
    /// 按级别索引的路由（快速查找）
    level_routes: HashMap<AlertLevel, RouteEntry>,
}

impl AlertRouter {
    /// 创建路由器
    pub fn new(config: RouterConfig) -> Self {
        let mut level_routes = HashMap::new();
        for entry in &config.routes {
            if let Some(ref level) = entry.level {
                level_routes.insert(level.clone(), entry.clone());
            }
        }

        Self {
            config,
            level_routes,
        }
    }

    /// 使用默认配置创建
    pub fn with_defaults() -> Self {
        Self::new(RouterConfig::default())
    }

    /// 为单个告警计算路由
    pub fn route(&self, alert: &Alert) -> RouteResult {
        // 先检查标签覆盖
        for label_route in &self.config.label_routes {
            if alert
                .labels
                .iter()
                .any(|(k, v)| k == &label_route.key && v == &label_route.value)
            {
                debug!(
                    alert_id = alert.id,
                    label_key = label_route.key,
                    label_value = label_route.value,
                    "label route override"
                );
                return RouteResult {
                    alert_id: alert.id.clone(),
                    channels: label_route.channels.clone(),
                    receivers: label_route.receivers.clone(),
                };
            }
        }

        // 按级别路由
        if let Some(entry) = self.level_routes.get(&alert.level) {
            RouteResult {
                alert_id: alert.id.clone(),
                channels: entry.channels.clone(),
                receivers: entry.receivers.clone(),
            }
        } else {
            // 未配置的级别，退回到 Warning 级别路由
            debug!(
                alert_id = alert.id,
                level = ?alert.level,
                "no route configured for level, falling back to Warning"
            );
            if let Some(entry) = self.level_routes.get(&AlertLevel::Warning) {
                RouteResult {
                    alert_id: alert.id.clone(),
                    channels: entry.channels.clone(),
                    receivers: entry.receivers.clone(),
                }
            } else {
                // 最终退回到空列表（不通知）
                RouteResult {
                    alert_id: alert.id.clone(),
                    channels: Vec::new(),
                    receivers: Vec::new(),
                }
            }
        }
    }

    /// 获取配置引用
    pub fn config(&self) -> &RouterConfig {
        &self.config
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use monitor_core::model::alert::{Alert, AlertLevel, AlertStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_alert(level: AlertLevel, labels: Vec<(&str, &str)>) -> Alert {
        Alert {
            id: Uuid::new_v4().to_string(),
            rule_name: "test".into(),
            level,
            status: AlertStatus::Firing,
            summary: "test".into(),
            description: "test".into(),
            service_name: "gateway".into(),
            fired_at: Utc::now(),
            acked_at: None,
            resolved_at: None,
            silenced_until: None,
            current_value: Some(1.0),
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            group_by: vec![],
        }
    }

    #[test]
    fn test_emergency_routes_to_phone_wecom_dingtalk() {
        let router = AlertRouter::with_defaults();
        let alert = make_alert(AlertLevel::Emergency, vec![]);
        let result = router.route(&alert);

        assert!(result.channels.contains(&Channel::Phone));
        assert!(result.channels.contains(&Channel::WeCom));
        assert!(result.channels.contains(&Channel::DingTalk));
        assert!(!result.channels.contains(&Channel::Email));
    }

    #[test]
    fn test_critical_routes_to_wecom_dingtalk_email() {
        let router = AlertRouter::with_defaults();
        let alert = make_alert(AlertLevel::Critical, vec![]);
        let result = router.route(&alert);

        assert!(result.channels.contains(&Channel::WeCom));
        assert!(result.channels.contains(&Channel::DingTalk));
        assert!(result.channels.contains(&Channel::Email));
        assert!(!result.channels.contains(&Channel::Phone));
    }

    #[test]
    fn test_warning_routes_to_email_only() {
        let router = AlertRouter::with_defaults();
        let alert = make_alert(AlertLevel::Warning, vec![]);
        let result = router.route(&alert);

        assert_eq!(result.channels, vec![Channel::Email]);
        assert_eq!(result.receivers, vec!["devops@hospital.com"]);
    }

    #[test]
    fn test_label_override() {
        let config = RouterConfig {
            label_routes: vec![LabelRoute {
                key: "service".into(),
                value: "payment".into(),
                channels: vec![Channel::Phone, Channel::Sms],
                receivers: vec!["payment-oncall".into()],
            }],
            ..Default::default()
        };

        let router = AlertRouter::new(config);

        // payment 服务的 Warning 级别应被标签覆盖为 Phone+Sms
        let alert = make_alert(
            AlertLevel::Warning,
            vec![("service", "payment")],
        );
        let result = router.route(&alert);
        assert!(result.channels.contains(&Channel::Phone));
        assert!(result.channels.contains(&Channel::Sms));
        assert_eq!(result.receivers, vec!["payment-oncall"]);

        // 其他服务仍走默认路由
        let alert2 = make_alert(AlertLevel::Warning, vec![("service", "gateway")]);
        let result2 = router.route(&alert2);
        assert_eq!(result2.channels, vec![Channel::Email]);
    }
}
