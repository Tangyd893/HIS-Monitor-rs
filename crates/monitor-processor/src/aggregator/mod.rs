//! 指标聚合与降采样
//!
//! 按时间窗口对指标样本进行聚合，支持 sum / avg / min / max / count 等聚合函数。
//! 聚合后的指标可用于降采样写入存储后端，减少存储压力。

use chrono::{DateTime, Utc};
use monitor_core::model::metric::MetricSample;
use std::collections::HashMap;
use tracing::debug;

/// 聚合函数类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    /// 求和
    Sum,
    /// 平均值
    Avg,
    /// 最小值
    Min,
    /// 最大值
    Max,
    /// 计数
    Count,
}

/// 聚合器配置
#[derive(Debug, Clone)]
pub struct AggregatorConfig {
    /// 时间窗口大小（秒）
    pub window_secs: i64,
    /// 窗口对齐（秒），0 表示自然对齐到 epoch 边界
    pub alignment_secs: i64,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            window_secs: 60,
            alignment_secs: 0,
        }
    }
}

/// 窗口键：按 (名称, 标签哈希) 分组
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WindowKey {
    name: String,
    /// 标签的排序后字符串表示（用于分组）
    labels_key: String,
    /// 窗口起始时间戳
    window_start: i64,
}

/// 窗口内累积桶
#[derive(Debug, Clone)]
struct WindowBucket {
    sum: f64,
    min: f64,
    max: f64,
    count: u64,
    /// 最后的值（用于 last 聚合）
    last: f64,
    /// 原始标签（用于输出）
    labels: Vec<(String, String)>,
    /// 窗口起始时间
    window_start: DateTime<Utc>,
}

impl WindowBucket {
    fn new(first_value: f64, labels: Vec<(String, String)>, window_start: DateTime<Utc>) -> Self {
        Self {
            sum: first_value,
            min: first_value,
            max: first_value,
            count: 1,
            last: first_value,
            labels,
            window_start,
        }
    }

    fn push(&mut self, value: f64) {
        self.sum += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.count += 1;
        self.last = value;
    }

    fn aggregate(&self, name: &str, kind: AggKind) -> AggregatedMetric {
        let value = match kind {
            AggKind::Sum => self.sum,
            AggKind::Avg => {
                if self.count > 0 {
                    self.sum / self.count as f64
                } else {
                    0.0
                }
            }
            AggKind::Min => self.min,
            AggKind::Max => self.max,
            AggKind::Count => self.count as f64,
        };

        AggregatedMetric {
            name: name.to_string(),
            labels: self.labels.clone(),
            value,
            window_start: self.window_start,
            sample_count: self.count,
        }
    }
}

/// 聚合后的指标
#[derive(Debug, Clone)]
pub struct AggregatedMetric {
    /// 原始指标名称（会追加聚合后缀如 `_sum`）
    pub name: String,
    /// 标签
    pub labels: Vec<(String, String)>,
    /// 聚合值
    pub value: f64,
    /// 窗口起始时间
    pub window_start: DateTime<Utc>,
    /// 窗口内原始样本数
    pub sample_count: u64,
}

/// 时间窗口聚合器
pub struct Aggregator {
    config: AggregatorConfig,
    windows: HashMap<WindowKey, WindowBucket>,
    /// 当前活跃窗口起始时间戳
    current_window_start: Option<i64>,
}

impl Aggregator {
    /// 创建新的聚合器
    pub fn new(config: AggregatorConfig) -> Self {
        Self {
            config,
            windows: HashMap::new(),
            current_window_start: None,
        }
    }

    /// 计算样本所属窗口起始时间戳（秒）
    fn window_start(&self, timestamp: DateTime<Utc>) -> i64 {
        let ts = timestamp.timestamp();
        let window_secs = self.config.window_secs;
        let alignment = self.config.alignment_secs;
        // 对齐到窗口边界
        let offset = if alignment > 0 {
            ((ts - alignment) % window_secs + window_secs) % window_secs
        } else {
            ts % window_secs
        };
        ts - offset
    }

    /// 将标签列表转换为排序后的字符串键
    fn labels_key(labels: &[(String, String)]) -> String {
        let mut sorted: Vec<_> = labels.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        sorted
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// 推送一个样本，如果窗口切换则返回上一窗口的聚合结果
    pub fn push(&mut self, sample: MetricSample) -> Vec<AggregatedMetric> {
        let ws = self.window_start(sample.timestamp);
        let key = WindowKey {
            name: sample.name.clone(),
            labels_key: Self::labels_key(&sample.labels),
            window_start: ws,
        };

        // 检测窗口切换
        let mut flushed = Vec::new();
        if let Some(current) = self.current_window_start {
            if ws != current {
                flushed = self.flush_window(current);
                debug!(
                    from = current,
                    to = ws,
                    metrics = flushed.len(),
                    "window advanced"
                );
            }
        }

        self.current_window_start = Some(ws);

        // 插入或更新桶
        match self.windows.get_mut(&key) {
            Some(bucket) => {
                bucket.push(sample.value);
            }
            None => {
                let window_dt = DateTime::from_timestamp(ws, 0)
                    .unwrap_or(sample.timestamp);
                self.windows.insert(
                    key,
                    WindowBucket::new(sample.value, sample.labels, window_dt),
                );
            }
        }

        flushed
    }

    /// 推送一批样本
    pub fn push_batch(&mut self, samples: Vec<MetricSample>) -> Vec<AggregatedMetric> {
        let mut all_flushed = Vec::new();
        for sample in samples {
            all_flushed.extend(self.push(sample));
        }
        all_flushed
    }

    /// 强制刷新指定窗口，返回聚合结果
    pub fn flush_window(&mut self, window_start: i64) -> Vec<AggregatedMetric> {
        let keys: Vec<_> = self
            .windows
            .keys()
            .filter(|k| k.window_start == window_start)
            .cloned()
            .collect();

        let mut results = Vec::new();
        for key in keys {
            if let Some(bucket) = self.windows.remove(&key) {
                // 默认输出 avg 聚合
                results.push(bucket.aggregate(&key.name, AggKind::Avg));
            }
        }

        results
    }

    /// 按指定聚合函数刷新窗口
    pub fn flush_window_with(
        &mut self,
        window_start: i64,
        kind: AggKind,
    ) -> Vec<AggregatedMetric> {
        let keys: Vec<_> = self
            .windows
            .keys()
            .filter(|k| k.window_start == window_start)
            .cloned()
            .collect();

        let mut results = Vec::new();
        for key in keys {
            if let Some(bucket) = self.windows.remove(&key) {
                results.push(bucket.aggregate(&key.name, kind));
            }
        }

        results
    }

    /// 同时输出多种聚合函数的结果
    pub fn flush_window_multi(
        &mut self,
        window_start: i64,
        kinds: &[AggKind],
    ) -> Vec<AggregatedMetric> {
        let keys: Vec<_> = self
            .windows
            .keys()
            .filter(|k| k.window_start == window_start)
            .cloned()
            .collect();

        let mut results = Vec::new();
        for key in keys {
            if let Some(bucket) = self.windows.get(&key) {
                for &kind in kinds {
                    let mut metric = bucket.aggregate(&key.name, kind);
                    // 追加聚合类型后缀
                    metric.name = format!("{}_{}", key.name, kind_label(kind));
                    results.push(metric);
                }
            }
        }

        // 清理已输出的窗口
        let remove_keys: Vec<_> = self
            .windows
            .keys()
            .filter(|k| k.window_start == window_start)
            .cloned()
            .collect();
        for key in remove_keys {
            self.windows.remove(&key);
        }

        results
    }

    /// 返回当前活跃窗口起始时间
    pub fn current_window(&self) -> Option<i64> {
        self.current_window_start
    }

    /// 当前缓存的窗口数
    pub fn pending_windows(&self) -> usize {
        self.windows.len()
    }
}

/// 聚合类型 → 标签后缀
fn kind_label(kind: AggKind) -> &'static str {
    match kind {
        AggKind::Sum => "sum",
        AggKind::Avg => "avg",
        AggKind::Min => "min",
        AggKind::Max => "max",
        AggKind::Count => "count",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_sample(name: &str, value: f64, ts: DateTime<Utc>, labels: Vec<(&str, &str)>) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            value,
            timestamp: ts,
        }
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn test_single_window_aggregation() {
        let mut agg = Aggregator::new(AggregatorConfig {
            window_secs: 60,
            alignment_secs: 0,
        });

        // 推送同一窗口内的多个样本
        let samples = vec![
            make_sample("cpu_usage", 0.5, ts(1001), vec![]),
            make_sample("cpu_usage", 0.7, ts(1002), vec![]),
            make_sample("cpu_usage", 0.6, ts(1003), vec![]),
        ];

        let flushed = agg.push_batch(samples);
        // 所有样本同一窗口，不应触发刷新
        assert!(flushed.is_empty());
        assert_eq!(agg.pending_windows(), 1);

        // 强制刷新
        let results = agg.flush_window(agg.window_start(ts(1000)));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "cpu_usage");
        assert_eq!(results[0].sample_count, 3);
        // avg = (0.5 + 0.7 + 0.6) / 3 = 0.6
        assert!((results[0].value - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_window_advance() {
        let mut agg = Aggregator::new(AggregatorConfig {
            window_secs: 60,
            alignment_secs: 0,
        });

        // 窗口 1: ts=0..60
        agg.push(make_sample("cpu", 0.5, ts(10), vec![]));
        agg.push(make_sample("cpu", 0.6, ts(20), vec![]));

        // 窗口 2: ts=60..120（触发窗口 1 刷新）
        let flushed = agg.push(make_sample("cpu", 1.0, ts(70), vec![]));
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].sample_count, 2);
        assert!((flushed[0].value - 0.55).abs() < 0.001);
        assert_eq!(flushed[0].window_start, ts(0));

        // 窗口 2 继续
        agg.push(make_sample("cpu", 1.2, ts(80), vec![]));
        assert_eq!(agg.pending_windows(), 1);
    }

    #[test]
    fn test_multi_window_flush() {
        let mut agg = Aggregator::new(AggregatorConfig {
            window_secs: 30,
            alignment_secs: 0,
        });

        agg.push(make_sample("cpu", 0.5, ts(0), vec![]));
        agg.push(make_sample("cpu", 0.6, ts(10), vec![]));
        agg.push(make_sample("cpu", 0.7, ts(40), vec![])); // 触发窗口 0-30 刷新

        assert_eq!(agg.pending_windows(), 1);
        let results = agg.flush_window(agg.window_start(ts(40)));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, 0.7);
    }

    #[test]
    fn test_label_grouping() {
        let mut agg = Aggregator::new(AggregatorConfig::default());

        agg.push(make_sample("http_req", 10.0, ts(0), vec![("method", "GET")]));
        agg.push(make_sample("http_req", 20.0, ts(5), vec![("method", "POST")]));
        agg.push(make_sample("http_req", 15.0, ts(10), vec![("method", "GET")]));

        let results = agg.flush_window(agg.window_start(ts(0)));
        assert_eq!(results.len(), 2);

        let get_result = results.iter().find(|r| r.labels.iter().any(|(k, v)| k == "method" && v == "GET")).unwrap();
        assert_eq!(get_result.sample_count, 2);
        assert_eq!(get_result.value, 12.5);

        let post_result = results.iter().find(|r| r.labels.iter().any(|(k, v)| k == "method" && v == "POST")).unwrap();
        assert_eq!(post_result.sample_count, 1);
        assert_eq!(post_result.value, 20.0);
    }

    #[test]
    fn test_multi_kind_flush() {
        let mut agg = Aggregator::new(AggregatorConfig::default());

        agg.push(make_sample("cpu", 0.2, ts(0), vec![]));
        agg.push(make_sample("cpu", 0.8, ts(5), vec![]));
        agg.push(make_sample("cpu", 0.5, ts(10), vec![]));

        let results = agg.flush_window_multi(
            agg.window_start(ts(0)),
            &[AggKind::Sum, AggKind::Avg, AggKind::Min, AggKind::Max, AggKind::Count],
        );

        assert_eq!(results.len(), 5);
        let by_name: HashMap<&str, f64> = results.iter().map(|m| (m.name.as_str(), m.value)).collect();
        assert!((by_name["cpu_sum"] - 1.5).abs() < 0.001);
        assert!((by_name["cpu_avg"] - 0.5).abs() < 0.001);
        assert!((by_name["cpu_min"] - 0.2).abs() < 0.001);
        assert!((by_name["cpu_max"] - 0.8).abs() < 0.001);
        assert!((by_name["cpu_count"] - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_alignment() {
        let agg = Aggregator::new(AggregatorConfig {
            window_secs: 60,
            alignment_secs: 0,
        });

        assert_eq!(agg.window_start(ts(0)), 0);
        assert_eq!(agg.window_start(ts(59)), 0);
        assert_eq!(agg.window_start(ts(60)), 60);
        assert_eq!(agg.window_start(ts(125)), 120);
    }
}
