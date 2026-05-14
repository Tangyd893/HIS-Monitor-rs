//! 基线学习模块
//!
//! 维护每个指标的历史统计数据（均值、标准差、同环比值），
//! 为异常检测提供参考基线。

use chrono::{DateTime, Duration, Utc};
use monitor_core::model::metric::MetricSample;
use std::collections::{HashMap, VecDeque};
use tracing::debug;

// ── 基线配置 ──

/// 基线配置
#[derive(Debug, Clone)]
pub struct BaselineConfig {
    /// 滑动窗口最大样本数
    pub max_samples: usize,
    /// 样本最大存活时间（秒），超时自动淘汰
    pub max_age_secs: i64,
    /// 同环比窗口大小（秒），用于计算前一周期对应时段
    pub compare_window_secs: i64,
}

impl Default for BaselineConfig {
    fn default() -> Self {
        Self {
            max_samples: 720,  // 3 小时 (15s 间隔)
            max_age_secs: 10800,
            compare_window_secs: 3600, // 对比前一小时
        }
    }
}

// ── 基线统计 ──

/// 单个指标的统计基线
#[derive(Debug, Clone)]
pub struct MetricBaseline {
    /// 指标名称
    pub name: String,
    /// 样本均值
    pub mean: f64,
    /// 样本标准差
    pub stddev: f64,
    /// 最小值
    pub min: f64,
    /// 最大值
    pub max: f64,
    /// 中位数（近似）
    pub median: f64,
    /// 样本数量
    pub count: usize,
    /// 最新值
    pub latest: f64,
    /// 上一周期同时段均值（用于同环比）
    pub prev_period_mean: Option<f64>,
    /// 基线更新时间
    pub updated_at: DateTime<Utc>,
}

// ── 内部样本存储 ──

/// 带时间戳的样本
#[derive(Debug, Clone)]
struct TimedSample {
    value: f64,
    timestamp: DateTime<Utc>,
}

/// 单个指标键的样本窗口
#[derive(Debug, Clone)]
struct SampleWindow {
    samples: VecDeque<TimedSample>,
}

impl SampleWindow {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }

    fn push(&mut self, value: f64, timestamp: DateTime<Utc>, max_samples: usize) {
        self.samples.push_back(TimedSample { value, timestamp });
        while self.samples.len() > max_samples {
            self.samples.pop_front();
        }
    }

    /// 淘汰过期样本
    fn expire(&mut self, max_age_secs: i64, now: DateTime<Utc>) {
        while let Some(front) = self.samples.front() {
            if (now - front.timestamp).num_seconds() > max_age_secs {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    fn compute_baseline(&self) -> Option<MetricBaseline> {
        if self.samples.is_empty() {
            return None;
        }

        let values: Vec<f64> = self.samples.iter().map(|s| s.value).collect();
        let count = values.len();
        let sum: f64 = values.iter().sum();
        let mean = sum / count as f64;
        let latest = values[count - 1];

        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
        let stddev = variance.sqrt();

        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        // 近似中位数
        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = if count % 2 == 0 {
            (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
        } else {
            sorted[count / 2]
        };

        Some(MetricBaseline {
            name: String::new(), // 由调用方填充
            mean,
            stddev,
            min,
            max,
            median,
            count,
            latest,
            prev_period_mean: None,
            updated_at: Utc::now(),
        })
    }

    /// 计算指定时间窗口内的均值
    fn window_mean(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Option<f64> {
        let window_values: Vec<f64> = self
            .samples
            .iter()
            .filter(|s| s.timestamp >= start && s.timestamp < end)
            .map(|s| s.value)
            .collect();

        if window_values.is_empty() {
            return None;
        }

        Some(window_values.iter().sum::<f64>() / window_values.len() as f64)
    }
}

// ── 基线管理器 ──

/// 指标键：name + labels 组合
type MetricKey = String;

/// 基线管理器
pub struct BaselineManager {
    config: BaselineConfig,
    /// 按指标键存储的样本窗口
    windows: HashMap<MetricKey, SampleWindow>,
    /// 最近一次过期检查时间
    last_expire: DateTime<Utc>,
}

impl BaselineManager {
    pub fn new(config: BaselineConfig) -> Self {
        Self {
            config,
            windows: HashMap::new(),
            last_expire: Utc::now(),
        }
    }

    /// 生成指标键
    pub fn metric_key(name: &str, labels: &[(String, String)]) -> MetricKey {
        let mut sorted: Vec<_> = labels.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let label_str: String = sorted
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(",");
        format!("{}::{}", name, label_str)
    }

    /// 推送一个样本
    pub fn push(&mut self, sample: &MetricSample) {
        let key = Self::metric_key(&sample.name, &sample.labels);
        let window = self
            .windows
            .entry(key)
            .or_insert_with(SampleWindow::new);

        window.push(sample.value, sample.timestamp, self.config.max_samples);
    }

    /// 批量推送样本
    pub fn push_batch(&mut self, samples: &[MetricSample]) {
        for sample in samples {
            self.push(sample);
        }
    }

    /// 获取某个指标的基线
    pub fn get_baseline(&mut self, name: &str, labels: &[(String, String)]) -> Option<MetricBaseline> {
        let key = Self::metric_key(name, labels);
        self.maybe_expire();

        let window = self.windows.get(&key)?;
        let mut baseline = window.compute_baseline()?;
        baseline.name = name.to_string();

        // 计算上一周期同时段的均值（同环比）
        let now = Utc::now();
        let period_start = now - Duration::seconds(self.config.compare_window_secs);
        let prev_start = period_start - Duration::seconds(self.config.compare_window_secs);

        baseline.prev_period_mean = window.window_mean(prev_start, period_start);

        Some(baseline)
    }

    /// 获取所有基线的快照
    pub fn all_baselines(&mut self) -> Vec<MetricBaseline> {
        self.maybe_expire();
        self.windows
            .iter()
            .map(|(key, window)| {
                let mut bl = window.compute_baseline();
                if let Some(ref mut b) = bl {
                    // 从 key 中提取指标名
                    b.name = key.split("::").next().unwrap_or(key).to_string();
                }
                bl
            })
            .flatten()
            .collect()
    }

    /// 当前追踪的指标数量
    pub fn metric_count(&self) -> usize {
        self.windows.len()
    }

    /// 定期过期检查
    fn maybe_expire(&mut self) {
        let now = Utc::now();
        // 每 60 秒执行一次过期检查
        if (now - self.last_expire).num_seconds() < 60 {
            return;
        }
        self.last_expire = now;

        let max_age = self.config.max_age_secs;
        let before = self.windows.len();

        // 清理空窗口
        self.windows.retain(|_, w| {
            w.expire(max_age, now);
            !w.samples.is_empty()
        });

        let after = self.windows.len();
        if before != after {
            debug!(before, after, "expired baseline windows");
        }
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_baseline_computation() {
        let mut window = SampleWindow::new();
        let now = Utc::now();

        // 推入 [1, 2, 3, 4, 5]
        for i in 1..=5 {
            window.push(i as f64, now, 100);
        }

        let baseline = window.compute_baseline().unwrap();
        assert_eq!(baseline.count, 5);
        assert!((baseline.mean - 3.0).abs() < 0.001);
        assert_eq!(baseline.min, 1.0);
        assert_eq!(baseline.max, 5.0);
        assert_eq!(baseline.median, 3.0);
        assert_eq!(baseline.latest, 5.0);
    }

    #[test]
    fn test_window_expiry() {
        let mut window = SampleWindow::new();
        let now = Utc::now();
        let old = now - Duration::seconds(200);

        window.push(1.0, old, 100);
        window.push(2.0, now, 100);

        assert_eq!(window.samples.len(), 2);
        window.expire(100, now); // 淘汰 100 秒前的
        assert_eq!(window.samples.len(), 1);
    }

    #[test]
    fn test_window_max_samples() {
        let mut window = SampleWindow::new();
        let now = Utc::now();

        for i in 0..10 {
            window.push(i as f64, now, 5); // max 5 samples
        }

        assert_eq!(window.samples.len(), 5);
        let baseline = window.compute_baseline().unwrap();
        assert!((baseline.mean - 7.0).abs() < 0.001); // avg of [5,6,7,8,9]
    }

    #[test]
    fn test_baseline_manager() {
        let config = BaselineConfig::default();
        let mut mgr = BaselineManager::new(config);
        let now = Utc::now();

        let samples: Vec<MetricSample> = (0..10)
            .map(|i| MetricSample {
                name: "cpu_usage".into(),
                labels: vec![("host".into(), "server1".into())],
                value: 50.0 + i as f64,
                timestamp: now,
            })
            .collect();

        mgr.push_batch(&samples);

        let baseline = mgr
            .get_baseline("cpu_usage", &[("host".into(), "server1".into())])
            .unwrap();

        assert_eq!(baseline.count, 10);
        assert!((baseline.mean - 54.5).abs() < 0.001);
        assert_eq!(baseline.name, "cpu_usage");
    }

    #[test]
    fn test_metric_key_stable() {
        let k1 = BaselineManager::metric_key("cpu", &[
            ("host".into(), "a".into()),
            ("env".into(), "prod".into()),
        ]);
        let k2 = BaselineManager::metric_key("cpu", &[
            ("env".into(), "prod".into()),
            ("host".into(), "a".into()),
        ]);
        assert_eq!(k1, k2);

        let k3 = BaselineManager::metric_key("cpu", &[
            ("host".into(), "b".into()),
        ]);
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_window_mean() {
        let mut window = SampleWindow::new();
        let t0 = Utc::now();

        // t0+10, t0+20, t0+30
        window.push(10.0, t0 + Duration::seconds(10), 100);
        window.push(20.0, t0 + Duration::seconds(20), 100);
        window.push(30.0, t0 + Duration::seconds(30), 100);

        let mean = window
            .window_mean(t0 + Duration::seconds(10), t0 + Duration::seconds(25))
            .unwrap();
        assert!((mean - 15.0).abs() < 0.001);
    }
}
