//! 告警数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 告警级别
///
/// 严重程度递增：Info < Warning < Critical < Emergency。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlertLevel {
    /// P3 - 信息
    Info,
    /// P2 - 警告
    Warning,
    /// P1 - 严重
    Critical,
    /// P0 - 紧急
    Emergency,
}

/// 告警状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertStatus {
    /// 待触发：规则已命中，等待持续时间满足
    Pending,
    /// 已触发：持续时间满足，已发送通知
    Firing,
    /// 已确认：运维人员已确认告警
    Acked,
    /// 已恢复：条件不再满足，自动恢复
    Resolved,
    /// 已静默：在静默窗口期间被抑制
    Silenced,
}

/// 告警记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub rule_name: String,
    pub level: AlertLevel,
    pub status: AlertStatus,
    pub summary: String,
    pub description: String,
    /// 产生告警的服务名称（从标签中提取或由规则指定）
    pub service_name: String,
    /// 首次触发时间（PENDING 开始时间）
    pub fired_at: DateTime<Utc>,
    /// 确认时间
    pub acked_at: Option<DateTime<Utc>>,
    /// 恢复时间
    pub resolved_at: Option<DateTime<Utc>>,
    /// 静默截止时间
    pub silenced_until: Option<DateTime<Utc>>,
    /// 当前指标值（最后一次评估时的值）
    pub current_value: Option<f64>,
    /// 告警附加标签
    pub labels: Vec<(String, String)>,
    /// 分组键（用于去重和聚合）
    #[serde(default)]
    pub group_by: Vec<String>,
}
