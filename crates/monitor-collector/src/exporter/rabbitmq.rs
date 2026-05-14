//! RabbitMQ 指标导出器
//!
//! 通过 RabbitMQ Management HTTP API 采集关键运行指标：
//! - 队列消息积压 (messages_ready / messages_unacknowledged)
//! - 死信队列 (dead_lettered)
//! - 连接数 / 通道数
//! - 内存与磁盘使用
//! - 消息发布/消费速率

use super::InfraExporter;
use chrono::Utc;
use monitor_core::model::metric::MetricSample;
use tracing::{debug, warn};

/// RabbitMQ Exporter
pub struct RabbitMqExporter {
    /// RabbitMQ Management API 地址 (http://host:15672)
    management_url: String,
    /// 用户名
    username: Option<String>,
    /// 密码
    password: Option<String>,
}

impl RabbitMqExporter {
    pub fn new(management_url: String, username: Option<String>, password: Option<String>) -> Self {
        Self {
            management_url,
            username,
            password,
        }
    }

    fn base_labels(&self) -> Vec<(String, String)> {
        vec![
            ("management_url".into(), self.management_url.clone()),
            ("exporter".into(), "rabbitmq_exporter".into()),
        ]
    }

    fn collect_queue_metrics(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "rabbitmq_messages_ready".into(),
                labels: labels.clone(),
                value: 256.0,
                timestamp: now,
            },
            MetricSample {
                name: "rabbitmq_messages_unacknowledged".into(),
                labels: labels.clone(),
                value: 12.0,
                timestamp: now,
            },
            MetricSample {
                name: "rabbitmq_messages_dead_lettered".into(),
                labels,
                value: 0.0,
                timestamp: now,
            },
        ]
    }

    fn collect_connections(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "rabbitmq_connections".into(),
                labels: labels.clone(),
                value: 34.0,
                timestamp: now,
            },
            MetricSample {
                name: "rabbitmq_channels".into(),
                labels,
                value: 128.0,
                timestamp: now,
            },
        ]
    }

    fn collect_resource_usage(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "rabbitmq_memory_used_bytes".into(),
                labels: labels.clone(),
                value: 268435456.0, // 256MB
                timestamp: now,
            },
            MetricSample {
                name: "rabbitmq_disk_free_bytes".into(),
                labels,
                value: 10737418240.0, // 10GB
                timestamp: now,
            },
        ]
    }

    fn collect_message_rates(&self) -> Vec<MetricSample> {
        let now = Utc::now();
        let labels = self.base_labels();

        vec![
            MetricSample {
                name: "rabbitmq_publish_rate".into(),
                labels: labels.clone(),
                value: 150.0,
                timestamp: now,
            },
            MetricSample {
                name: "rabbitmq_deliver_rate".into(),
                labels,
                value: 148.0,
                timestamp: now,
            },
        ]
    }
}

#[async_trait::async_trait]
impl InfraExporter for RabbitMqExporter {
    fn name(&self) -> &str {
        "rabbitmq_exporter"
    }

    async fn collect(&self) -> anyhow::Result<Vec<MetricSample>> {
        debug!("collecting RabbitMQ metrics from {}", self.management_url);

        let mut metrics = Vec::new();
        metrics.extend(self.collect_queue_metrics());
        metrics.extend(self.collect_connections());
        metrics.extend(self.collect_resource_usage());
        metrics.extend(self.collect_message_rates());

        warn!(
            url = self.management_url,
            count = metrics.len(),
            "RabbitMqExporter returned mock data — implement real HTTP API calls"
        );
        Ok(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rabbitmq_exporter_returns_mock_metrics() {
        let exporter = RabbitMqExporter::new(
            "http://localhost:15672".into(),
            Some("admin".into()),
            Some("admin".into()),
        );
        let metrics = exporter.collect().await.unwrap();

        assert!(!metrics.is_empty());
        assert!(metrics.iter().any(|m| m.name == "rabbitmq_messages_ready"));
        assert!(metrics.iter().any(|m| m.name == "rabbitmq_connections"));
        assert!(metrics.iter().any(|m| m.name == "rabbitmq_memory_used_bytes"));
        assert!(metrics.iter().any(|m| m.name == "rabbitmq_publish_rate"));
    }
}
