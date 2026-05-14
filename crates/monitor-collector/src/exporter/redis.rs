//! Redis 指标导出器
//!
//! 通过 Redis INFO 命令采集关键运行指标：
//! - 内存使用（used_memory_rss / maxmemory）
//! - 命中率（keyspace_hits / keyspace_misses）
//! - 连接数（connected_clients / maxclients）
//! - 命令执行速率
//! - 主从复制状态

use super::InfraExporter;
use chrono::Utc;
use monitor_core::model::metric::MetricSample;
use tracing::{debug, warn};

/// Redis Exporter
pub struct RedisExporter {
    /// Redis 连接 URL (redis://host:port)
    url: String,
}

impl RedisExporter {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    fn base_labels(&self) -> Vec<(String, String)> {
        vec![
            ("url".into(), self.url.clone()),
            ("exporter".into(), "redis_exporter".into()),
        ]
    }

    fn collect_memory(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "redis_memory_used_bytes".into(),
                labels: labels.clone(),
                value: 524288000.0, // 500MB
                timestamp: now,
            },
            MetricSample {
                name: "redis_memory_max_bytes".into(),
                labels,
                value: 1073741824.0, // 1GB
                timestamp: now,
            },
        ]
    }

    fn collect_hit_rate(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![MetricSample {
            name: "redis_keyspace_hit_rate".into(),
            labels,
            value: 0.95,
            timestamp: now,
        }]
    }

    fn collect_connections(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "redis_connected_clients".into(),
                labels: labels.clone(),
                value: 42.0,
                timestamp: now,
            },
            MetricSample {
                name: "redis_max_clients".into(),
                labels,
                value: 10000.0,
                timestamp: now,
            },
        ]
    }

    fn collect_replication(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![MetricSample {
            name: "redis_replication_offset_diff".into(),
            labels,
            value: 0.0,
            timestamp: now,
        }]
    }

    fn collect_ops(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![MetricSample {
            name: "redis_instantaneous_ops_per_sec".into(),
            labels,
            value: 2500.0,
            timestamp: now,
        }]
    }
}

#[async_trait::async_trait]
impl InfraExporter for RedisExporter {
    fn name(&self) -> &str {
        "redis_exporter"
    }

    async fn collect(&self) -> anyhow::Result<Vec<MetricSample>> {
        debug!("collecting Redis metrics for {}", self.url);

        let mut metrics = Vec::new();
        metrics.extend(self.collect_memory());
        metrics.extend(self.collect_hit_rate());
        metrics.extend(self.collect_connections());
        metrics.extend(self.collect_replication());
        metrics.extend(self.collect_ops());

        warn!(
            url = self.url,
            count = metrics.len(),
            "RedisExporter returned mock data — implement real Redis INFO parsing"
        );
        Ok(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_redis_exporter_returns_mock_metrics() {
        let exporter = RedisExporter::new("redis://localhost:6379".into());
        let metrics = exporter.collect().await.unwrap();

        assert!(!metrics.is_empty());
        assert!(metrics.iter().any(|m| m.name == "redis_memory_used_bytes"));
        assert!(metrics.iter().any(|m| m.name == "redis_keyspace_hit_rate"));
        assert!(metrics.iter().any(|m| m.name == "redis_connected_clients"));
        assert!(metrics.iter().any(|m| m.name == "redis_instantaneous_ops_per_sec"));
    }
}
