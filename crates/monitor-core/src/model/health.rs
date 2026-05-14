//! 健康检查数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 健康检查状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    Up,
    Down,
    Degraded,
}

/// 健康检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub target: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub latency_ms: u64,
}
