//! 指标数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 指标样本
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    /// 指标名称
    pub name: String,
    /// 标签键值对
    pub labels: Vec<(String, String)>,
    /// 指标值
    pub value: f64,
    /// 采集时间戳
    pub timestamp: DateTime<Utc>,
}
