//! monitor-processor — 数据处理引擎
//!
//! 负责指标聚合降采样、告警规则评估引擎、异常检测。

pub mod aggregator;
pub mod anomaly;
pub mod stream;
