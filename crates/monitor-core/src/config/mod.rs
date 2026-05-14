//! 配置加载模块
//!
//! 支持本地 YAML 文件（config-rs）和 Nacos 远程配置中心。

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

/// 监控系统顶层配置
#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    /// 采集配置
    pub collector: CollectorConfig,
    /// 处理配置
    pub processor: ProcessorConfig,
    /// 告警配置
    pub alert: AlertConfig,
    /// API 服务配置
    pub api: ApiConfig,
    /// 探活配置
    pub probe: ProbeConfig,
}

/// 采集配置
#[derive(Debug, Clone, Deserialize)]
pub struct CollectorConfig {
    /// 默认采集间隔（秒）
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    /// 采集目标列表
    #[serde(default)]
    pub targets: Vec<ScrapeTarget>,
    /// HTTP 请求超时（秒）
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

/// 单个抓取目标
#[derive(Debug, Clone, Deserialize)]
pub struct ScrapeTarget {
    /// 目标名称（服务名）
    pub name: String,
    /// /metrics 端点 URL
    pub url: String,
    /// 目标采集间隔（秒），为空则使用全局默认值
    pub interval_secs: Option<u64>,
    /// HIS 业务标签（富化）
    #[serde(default)]
    pub labels: Vec<(String, String)>,
}

/// 处理配置
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessorConfig {
    /// 批量写入大小
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// 是否启用指标采样
    #[serde(default)]
    pub sample_enabled: bool,
    /// 采样率（每 N 个样本保留 1 个）
    #[serde(default = "default_sample_rate")]
    pub sample_rate: usize,
}

/// 告警配置
#[derive(Debug, Clone, Deserialize)]
pub struct AlertConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// API 配置
#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

/// 探活配置
#[derive(Debug, Clone, Deserialize)]
pub struct ProbeConfig {
    #[serde(default = "default_probe_interval")]
    pub interval_secs: u64,
}

// ── 默认值 ──

fn default_interval() -> u64 { 15 }
fn default_timeout() -> u64 { 10 }
fn default_batch_size() -> usize { 1000 }
fn default_sample_rate() -> usize { 4 }
fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 9100 }
fn default_probe_interval() -> u64 { 30 }

impl MonitorConfig {
    /// 从本地文件和环境变量加载配置
    pub fn load(run_mode: &str) -> Result<Self, ConfigError> {
        let config = Config::builder()
            .add_source(File::with_name("configs/monitor").required(false))
            .add_source(File::with_name(&format!("configs/monitor.{}", run_mode)).required(false))
            .add_source(Environment::with_prefix("MONITOR").separator("__"))
            .build()?;

        config.try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let yaml = r#"
collector:
  interval_secs: 15
  targets:
    - name: "gateway"
      url: "http://localhost:8080/metrics"
    - name: "auth"
      url: "http://localhost:8081/metrics"
      interval_secs: 30
      labels:
        - ["team", "iam"]
processor:
  batch_size: 1000
alert:
  enabled: true
api:
  host: "0.0.0.0"
  port: 9100
probe:
  interval_secs: 30
"#;
        let cfg: MonitorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.collector.targets.len(), 2);
        assert_eq!(cfg.collector.targets[0].name, "gateway");
        assert_eq!(cfg.collector.targets[1].interval_secs, Some(30));
        assert_eq!(cfg.collector.targets[1].labels[0], ("team".into(), "iam".into()));
    }

    #[test]
    fn test_minimal_config() {
        let yaml = "collector: {}\nprocessor: {}\nalert: {}\napi: {}\nprobe: {}";
        let cfg: MonitorConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.collector.interval_secs, 15);
        assert!(cfg.collector.targets.is_empty());
    }
}
