//! 追踪数据模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 追踪 Span
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub service_name: String,
    pub operation_name: String,
    pub start_time: DateTime<Utc>,
    pub duration_ms: u64,
    pub status_code: i32,
    pub tags: Vec<(String, String)>,
}
