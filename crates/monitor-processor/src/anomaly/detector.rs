//! 异常检测算法
//!
//! 基于历史基线检测指标异常，支持四种策略：
//! - 固定阈值 (FixedThreshold)
//! - 同环比波动 (PeriodOverPeriod)
//! - 标准差偏离 (StdDeviation)
//! - 突变检测 (SuddenChange)

use super::baseline::{BaselineConfig, BaselineManager, MetricBaseline};
use monitor_core::model::metric::MetricSample;
use serde::{Deserialize, Serialize};

// ── 检测策略 ──

/// 异常检测策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetectionStrategy {
    /// 固定阈值：value > max 或 value < min
    FixedThreshold {
        /// 上限阈值（超过则异常）
        max: Option<f64>,
        /// 下限阈值（低于则异常）
        min: Option<f64>,
    },
    /// 同环比波动：当前值相比历史同时段均值的变化率
    PeriodOverPeriod {
        /// 变化率阈值（如 0.5 表示下降超过 50%）
        change_rate: f64,
        /// 方向：up（上升异常）、down（下降异常）、both
        direction: ChangeDirection,
    },
    /// 标准差偏离：当前值偏离均值超过 N 倍标准差
    StdDeviation {
        /// 标准差倍数
        multiplier: f64,
        /// 最低样本数要求（低于此数不检测）
        min_samples: usize,
    },
    /// 突变检测：短时间窗口内变化率超过阈值
    SuddenChange {
        /// 变化率阈值（如 2.0 表示变化超过 200%）
        change_rate: f64,
        /// 检测窗口大小（秒）
        window_secs: i64,
        /// 最低样本数要求
        min_samples: usize,
    },
}

/// 变化方向
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeDirection {
    Up,
    Down,
    Both,
}

// ── 检测结果 ──

/// 异常检测结果
#[derive(Debug, Clone)]
pub struct AnomalyResult {
    /// 指标名称
    pub metric_name: String,
    /// 标签
    pub labels: Vec<(String, String)>,
    /// 当前值
    pub current_value: f64,
    /// 触发的策略
    pub strategy: String,
    /// 异常级别：warning / critical
    pub severity: AnomalySeverity,
    /// 描述
    pub description: String,
}

/// 异常严重级别
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomalySeverity {
    Warning,
    Critical,
}

// ── 异常检测器 ──

/// 异常检测器
pub struct AnomalyDetector {
    /// 基线管理器
    pub baseline: BaselineManager,
    /// 检测策略列表（按指标名匹配）
    strategies: Vec<(String, DetectionStrategy)>,
}

impl AnomalyDetector {
    pub fn new(config: BaselineConfig) -> Self {
        Self {
            baseline: BaselineManager::new(config),
            strategies: Vec::new(),
        }
    }

    /// 为指定指标名添加检测策略
    pub fn add_strategy(&mut self, metric_pattern: &str, strategy: DetectionStrategy) {
        self.strategies
            .push((metric_pattern.to_string(), strategy));
    }

    /// 检测单个样本是否异常
    pub fn detect(&mut self, sample: &MetricSample) -> Vec<AnomalyResult> {
        let mut results = Vec::new();

        for (pattern, strategy) in &self.strategies {
            if !metric_name_matches(pattern, &sample.name) {
                continue;
            }

            let baseline = self.baseline.get_baseline(&sample.name, &sample.labels);
            let anomaly = match strategy {
                DetectionStrategy::FixedThreshold { max, min } => {
                    detect_fixed_threshold(sample, *max, *min)
                }
                DetectionStrategy::PeriodOverPeriod {
                    change_rate,
                    direction,
                } => detect_period_over_period(sample, baseline.as_ref(), *change_rate, direction),
                DetectionStrategy::StdDeviation {
                    multiplier,
                    min_samples,
                } => detect_stddev(sample, baseline.as_ref(), *multiplier, *min_samples),
                DetectionStrategy::SuddenChange {
                    change_rate,
                    window_secs,
                    min_samples,
                } => detect_sudden_change(sample, baseline.as_ref(), *change_rate, *window_secs, *min_samples),
            };

            if let Some(mut result) = anomaly {
                result.strategy = format_strategy(strategy);
                results.push(result);
            }
        }

        // 推送样本到基线（供后续检测使用）
        self.baseline.push(sample);

        results
    }

    /// 批量检测
    pub fn detect_batch(&mut self, samples: &[MetricSample]) -> Vec<AnomalyResult> {
        let mut results = Vec::new();
        for sample in samples {
            results.extend(self.detect(sample));
        }
        results
    }
}

// ── 检测函数 ──

/// 固定阈值检测
fn detect_fixed_threshold(
    sample: &MetricSample,
    max: Option<f64>,
    min: Option<f64>,
) -> Option<AnomalyResult> {
    let mut triggered = false;
    let mut desc = String::new();

    if let Some(max_val) = max {
        if sample.value > max_val {
            triggered = true;
            desc = format!("value {} exceeds max {}", sample.value, max_val);
        }
    }

    if let Some(min_val) = min {
        if sample.value < min_val {
            triggered = true;
            desc = format!("value {} below min {}", sample.value, min_val);
        }
    }

    if triggered {
        Some(AnomalyResult {
            metric_name: sample.name.clone(),
            labels: sample.labels.clone(),
            current_value: sample.value,
            strategy: String::new(),
            severity: AnomalySeverity::Warning,
            description: desc,
        })
    } else {
        None
    }
}

/// 同环比波动检测
fn detect_period_over_period(
    sample: &MetricSample,
    baseline: Option<&MetricBaseline>,
    change_rate: f64,
    direction: &ChangeDirection,
) -> Option<AnomalyResult> {
    let bl = baseline?;
    let prev = bl.prev_period_mean?;

    if prev == 0.0 {
        return None;
    }

    let actual_change = (sample.value - prev) / prev;
    let direction_match = match direction {
        ChangeDirection::Up => actual_change > change_rate && actual_change > 0.0,
        ChangeDirection::Down => actual_change < -change_rate && actual_change < 0.0,
        ChangeDirection::Both => actual_change.abs() > change_rate,
    };

    if direction_match {
        let desc = format!(
            "current value {} vs prev period mean {}, change {:.1}%",
            sample.value,
            prev,
            actual_change * 100.0
        );
        let severity = if actual_change.abs() > change_rate * 2.0 {
            AnomalySeverity::Critical
        } else {
            AnomalySeverity::Warning
        };

        Some(AnomalyResult {
            metric_name: sample.name.clone(),
            labels: sample.labels.clone(),
            current_value: sample.value,
            strategy: String::new(),
            severity,
            description: desc,
        })
    } else {
        None
    }
}

/// 标准差偏离检测
fn detect_stddev(
    sample: &MetricSample,
    baseline: Option<&MetricBaseline>,
    multiplier: f64,
    min_samples: usize,
) -> Option<AnomalyResult> {
    let bl = baseline?;

    if bl.count < min_samples || bl.stddev == 0.0 {
        return None;
    }

    let deviation = (sample.value - bl.mean).abs() / bl.stddev;

    if deviation > multiplier {
        let desc = format!(
            "value {} deviates {:.1}σ from mean {} (stddev {})",
            sample.value, deviation, bl.mean, bl.stddev
        );
        let severity = if deviation > multiplier * 2.0 {
            AnomalySeverity::Critical
        } else {
            AnomalySeverity::Warning
        };

        Some(AnomalyResult {
            metric_name: sample.name.clone(),
            labels: sample.labels.clone(),
            current_value: sample.value,
            strategy: String::new(),
            severity,
            description: desc,
        })
    } else {
        None
    }
}

/// 突变检测（短窗口变化率）
fn detect_sudden_change(
    sample: &MetricSample,
    baseline: Option<&MetricBaseline>,
    change_rate: f64,
    _window_secs: i64,
    min_samples: usize,
) -> Option<AnomalyResult> {
    let bl = baseline?;

    if bl.count < min_samples || bl.latest == 0.0 {
        return None;
    }

    // 对比最新值与均值的变化率
    let change = (sample.value - bl.latest).abs() / bl.latest;

    if change > change_rate {
        let direction = if sample.value > bl.latest { "increase" } else { "decrease" };
        let desc = format!(
            "sudden {}: {} -> {} ({:.1}% change)",
            direction,
            bl.latest,
            sample.value,
            change * 100.0
        );

        Some(AnomalyResult {
            metric_name: sample.name.clone(),
            labels: sample.labels.clone(),
            current_value: sample.value,
            strategy: String::new(),
            severity: AnomalySeverity::Critical,
            description: desc,
        })
    } else {
        None
    }
}

/// 指标名匹配
fn metric_name_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

/// 策略名称格式化
fn format_strategy(strategy: &DetectionStrategy) -> String {
    match strategy {
        DetectionStrategy::FixedThreshold { .. } => "fixed_threshold".into(),
        DetectionStrategy::PeriodOverPeriod { change_rate, direction } => {
            format!("period_over_period(rate={},dir={:?})", change_rate, direction)
        }
        DetectionStrategy::StdDeviation { multiplier, .. } => {
            format!("stddev(multiplier={})", multiplier)
        }
        DetectionStrategy::SuddenChange { change_rate, .. } => {
            format!("sudden_change(rate={})", change_rate)
        }
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_sample(name: &str, value: f64) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: vec![],
            value,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_fixed_threshold_max() {
        let sample = make_sample("cpu_usage", 0.95);
        let result = detect_fixed_threshold(&sample, Some(0.9), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().severity, AnomalySeverity::Warning);
    }

    #[test]
    fn test_fixed_threshold_ok() {
        let sample = make_sample("cpu_usage", 0.5);
        let result = detect_fixed_threshold(&sample, Some(0.9), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_fixed_threshold_min() {
        let sample = make_sample("qps", 10.0);
        let result = detect_fixed_threshold(&sample, None, Some(50.0));
        assert!(result.is_some());
    }

    #[test]
    fn test_stddev_detection() {
        let mut detector = AnomalyDetector::new(BaselineConfig::default());
        detector.add_strategy(
            "cpu_usage",
            DetectionStrategy::StdDeviation {
                multiplier: 3.0,
                min_samples: 5,
            },
        );

        let now = Utc::now();

        // 推入正常样本：均值 ≈ 50, stddev ≈ 2
        for i in 0..10 {
            detector.baseline.push(&MetricSample {
                name: "cpu_usage".into(),
                labels: vec![],
                value: 48.0 + (i as f64 % 5.0),
                timestamp: now,
            });
        }

        // 正常值不应触发
        let normal = make_sample("cpu_usage", 51.0);
        let results = detector.detect(&normal);
        assert!(results.is_empty());

        // 异常值应触发（偏离 > 3σ）
        let anomaly = make_sample("cpu_usage", 70.0);
        let results = detector.detect(&anomaly);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_sudden_change() {
        let mut detector = AnomalyDetector::new(BaselineConfig::default());
        detector.add_strategy(
            "traffic",
            DetectionStrategy::SuddenChange {
                change_rate: 2.0,
                window_secs: 300,
                min_samples: 3,
            },
        );

        let now = Utc::now();

        // 推入稳定流量
        for i in 0..5 {
            detector.baseline.push(&MetricSample {
                name: "traffic".into(),
                labels: vec![],
                value: 100.0 + i as f64,
                timestamp: now,
            });
        }

        // 突然激增 3 倍
        let spike = make_sample("traffic", 400.0);
        let results = detector.detect(&spike);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].severity, AnomalySeverity::Critical);
    }

    #[test]
    fn test_no_baseline_no_detection() {
        let mut detector = AnomalyDetector::new(BaselineConfig::default());
        detector.add_strategy(
            "cpu_usage",
            DetectionStrategy::StdDeviation {
                multiplier: 3.0,
                min_samples: 10,
            },
        );

        // 没有足够样本，不应触发
        let sample = make_sample("cpu_usage", 999.0);
        let results = detector.detect(&sample);
        assert!(results.is_empty());
    }
}
