//! 异常检测模块
//!
//! 包含基线学习和四种异常检测算法：
//! - 固定阈值 (FixedThreshold)
//! - 同环比波动 (PeriodOverPeriod)
//! - 标准差偏离 (StdDeviation)
//! - 突变检测 (SuddenChange)

pub mod baseline;
pub mod detector;

pub use baseline::{BaselineConfig, BaselineManager, MetricBaseline};
pub use detector::{AnomalyDetector, AnomalyResult, AnomalySeverity, ChangeDirection, DetectionStrategy};
