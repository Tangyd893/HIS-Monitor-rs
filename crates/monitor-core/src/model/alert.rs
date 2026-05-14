//! 告警数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 告警级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertLevel {
    /// P0 - 紧急
    Emergency,
    /// P1 - 严重
    Critical,
    /// P2 - 警告
    Warning,
    /// P3 - 信息
    Info,
}

/// 告警状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertStatus {
    Firing,
    Resolved,
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
    pub service_name: String,
    pub fired_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub labels: Vec<(String, String)>,
}
