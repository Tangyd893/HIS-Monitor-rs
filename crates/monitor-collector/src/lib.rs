//! monitor-collector — 数据采集器
//!
//! 负责 Prometheus 指标抓取、OTLP 追踪接收、日志采集
//! 以及 PostgreSQL / Redis / RabbitMQ / Nacos / MinIO 基础设施 Exporter。

pub mod exporter;
pub mod pipeline;
pub mod receiver;
pub mod scraper;
