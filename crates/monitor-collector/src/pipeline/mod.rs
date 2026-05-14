//! 采集流水线
//!
//! 对抓取到的指标样本进行：过滤 → 变换 → 富化 → 采样。
//! 采用构建器模式，链式组合各阶段处理器。

use monitor_core::model::metric::MetricSample;
use monitor_core::config::ProcessorConfig;

/// 流水线处理器
#[derive(Clone)]
pub struct Pipeline {
    config: ProcessorConfig,
    /// 额外注入的标签（如 HIS 业务标签）
    extra_labels: Vec<(String, String)>,
    /// 指标名称白名单（为空表示不过滤）
    name_allowlist: Vec<String>,
    /// 指标名称黑名单
    name_blocklist: Vec<String>,
}

impl Pipeline {
    /// 创建默认流水线
    pub fn new(config: ProcessorConfig) -> Self {
        Self {
            config,
            extra_labels: Vec::new(),
            name_allowlist: Vec::new(),
            name_blocklist: Vec::new(),
        }
    }

    /// 注入额外标签
    pub fn with_labels(mut self, labels: Vec<(String, String)>) -> Self {
        self.extra_labels = labels;
        self
    }

    /// 设置指标名称白名单
    pub fn with_allowlist(mut self, allowlist: Vec<String>) -> Self {
        self.name_allowlist = allowlist;
        self
    }

    /// 设置指标名称黑名单
    pub fn with_blocklist(mut self, blocklist: Vec<String>) -> Self {
        self.name_blocklist = blocklist;
        self
    }

    /// 处理一批样本
    pub fn process(&self, samples: Vec<MetricSample>) -> Vec<MetricSample> {
        let samples = self.filter(samples);
        let samples = self.enrich(samples);
        self.sample(samples)
    }

    /// 过滤阶段：白名单/黑名单
    fn filter(&self, samples: Vec<MetricSample>) -> Vec<MetricSample> {
        if self.name_allowlist.is_empty() && self.name_blocklist.is_empty() {
            return samples;
        }

        samples
            .into_iter()
            .filter(|s| {
                // 黑名单优先
                if self.name_blocklist.iter().any(|n| n == &s.name) {
                    return false;
                }
                // 白名单：命中则通过，列表为空则全通过
                self.name_allowlist.is_empty()
                    || self.name_allowlist.iter().any(|n| n == &s.name)
            })
            .collect()
    }

    /// 富化阶段：注入 HIS 业务标签
    fn enrich(&self, samples: Vec<MetricSample>) -> Vec<MetricSample> {
        if self.extra_labels.is_empty() {
            return samples;
        }

        samples
            .into_iter()
            .map(|mut s| {
                s.labels.extend(self.extra_labels.clone());
                s
            })
            .collect()
    }

    /// 采样阶段：保留每 N 个中的第 1 个
    fn sample(&self, samples: Vec<MetricSample>) -> Vec<MetricSample> {
        if !self.config.sample_enabled || self.config.sample_rate <= 1 {
            return samples;
        }

        samples
            .into_iter()
            .enumerate()
            .filter(|(i, _)| i % self.config.sample_rate == 0)
            .map(|(_, s)| s)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_sample(name: &str, value: f64) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: Vec::new(),
            value,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_filter_allowlist() {
        let pipeline = Pipeline::new(ProcessorConfig {
            batch_size: 1000,
            sample_enabled: false,
            sample_rate: 4,
        })
        .with_allowlist(vec!["cpu_usage".into()]);

        let samples = vec![
            make_sample("cpu_usage", 0.5),
            make_sample("mem_usage", 0.8),
            make_sample("cpu_usage", 0.6),
        ];

        let result = pipeline.process(samples);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "cpu_usage");
    }

    #[test]
    fn test_filter_blocklist() {
        let pipeline = Pipeline::new(ProcessorConfig {
            batch_size: 1000,
            sample_enabled: false,
            sample_rate: 4,
        })
        .with_blocklist(vec!["go_gc_duration_seconds".into()]);

        let samples = vec![
            make_sample("cpu_usage", 0.5),
            make_sample("go_gc_duration_seconds", 0.01),
        ];

        let result = pipeline.process(samples);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_enrich_labels() {
        let pipeline = Pipeline::new(ProcessorConfig {
            batch_size: 1000,
            sample_enabled: false,
            sample_rate: 4,
        })
        .with_labels(vec![("service".into(), "gateway".into())]);

        let samples = vec![make_sample("cpu_usage", 0.5)];
        let result = pipeline.process(samples);

        assert_eq!(result[0].labels.len(), 1);
        assert_eq!(result[0].labels[0], ("service".into(), "gateway".into()));
    }

    #[test]
    fn test_sample_enabled() {
        let pipeline = Pipeline::new(ProcessorConfig {
            batch_size: 1000,
            sample_enabled: true,
            sample_rate: 3,
        });

        let samples: Vec<_> = (0..10)
            .map(|i| make_sample("cpu_usage", i as f64))
            .collect();

        let result = pipeline.process(samples);
        // 10 / 3 ≈ 4, indices 0,3,6,9
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].value, 0.0);
        assert_eq!(result[1].value, 3.0);
        assert_eq!(result[2].value, 6.0);
        assert_eq!(result[3].value, 9.0);
    }

    #[test]
    fn test_no_sample_when_disabled() {
        let pipeline = Pipeline::new(ProcessorConfig {
            batch_size: 1000,
            sample_enabled: false,
            sample_rate: 3,
        });

        let samples: Vec<_> = (0..10)
            .map(|i| make_sample("cpu_usage", i as f64))
            .collect();

        let result = pipeline.process(samples);
        assert_eq!(result.len(), 10);
    }
}
