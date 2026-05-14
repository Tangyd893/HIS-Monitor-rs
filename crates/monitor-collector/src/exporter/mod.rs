//! 基础设施 Exporter
//!
//! 为 HIS-Go 依赖的 PostgreSQL、Redis、RabbitMQ 等基础设施提供指标采集能力。
//! 每个 Exporter 实现 [InfraExporter] trait，输出 [MetricSample] 流。

pub mod postgres;
pub mod redis;
pub mod rabbitmq;

use async_trait::async_trait;
use monitor_core::model::metric::MetricSample;

/// 基础设施 Exporter 抽象接口
#[async_trait]
pub trait InfraExporter: Send + Sync {
    /// Exporter 名称
    fn name(&self) -> &str;

    /// 采集一次指标
    async fn collect(&self) -> anyhow::Result<Vec<MetricSample>>;
}
