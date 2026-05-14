//! OTLP gRPC 追踪接收器
//!
//! 基于 Tonic 接收 OpenTelemetry Protocol (OTLP) 的 Trace 数据，
//! 实现 `TraceService` gRPC 服务，接收 Span 后写入处理管道。

use monitor_core::model::trace::TraceSpan;
use monitor_core::model::metric::MetricSample;
use chrono::Utc;
use tracing::{debug, info};

/// OTLP 接收器配置
#[derive(Debug, Clone)]
pub struct OltpReceiverConfig {
    /// 监听地址
    pub listen_addr: String,
    /// 最大消息大小（字节）
    pub max_message_size: usize,
}

impl Default for OltpReceiverConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:4317".into(),
            max_message_size: 16 * 1024 * 1024, // 16MB
        }
    }
}

/// OTLP 追踪接收器
///
/// 注：完整的 OTLP TraceService 需要编译 `opentelemetry-proto` 的 protobuf 定义。
/// 当前提供骨架结构：接收 Span 分组并转换为内部 [TraceSpan] 模型。
pub struct OltpReceiver {
    config: OltpReceiverConfig,
}

impl OltpReceiver {
    pub fn new(config: OltpReceiverConfig) -> Self {
        Self { config }
    }

    /// 处理接收到的 ResourceSpan 批次
    pub fn process_spans(&self, spans: Vec<TraceSpan>) -> (Vec<TraceSpan>, Vec<MetricSample>) {
        let now = Utc::now();
        let mut metrics = Vec::new();

        // 按服务名分组统计 Span 数量
        let service_counts: std::collections::HashMap<String, usize> =
            spans.iter().fold(Default::default(), |mut acc, s| {
                *acc.entry(s.service_name.clone()).or_default() += 1;
                acc
            });

        for (service, count) in &service_counts {
            metrics.push(MetricSample {
                name: "otel_spans_received_total".into(),
                labels: vec![
                    ("service".into(), service.clone()),
                    ("receiver".into(), "otlp".into()),
                ],
                value: *count as f64,
                timestamp: now,
            });
        }

        debug!(
            spans = spans.len(),
            services = service_counts.len(),
            "received spans"
        );

        (spans, metrics)
    }
}

/// 启动 OTLP gRPC 服务
///
/// 当前为占位实现，需要：
/// 1. 在 `monitor-core/protocol/proto/` 中添加 `opentelemetry.proto`
/// 2. 通过 `tonic-build` 编译生成 Rust 代码
/// 3. 实现 `TraceServiceServer`
pub async fn run_receiver(config: OltpReceiverConfig) -> anyhow::Result<()> {
    info!(
        addr = config.listen_addr,
        "OTLP receiver starting (stub)"
    );

    // TODO: 完整实现
    // let addr = config.listen_addr.parse()?;
    // let svc = TraceServiceServer::new(OtlpTraceService::new());
    // Server::builder()
    //     .add_service(svc)
    //     .serve(addr)
    //     .await?;

    info!("OTLP receiver stub — full implementation requires protobuf compilation");
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_spans_generates_metrics() {
        let receiver = OltpReceiver::new(OltpReceiverConfig::default());
        let spans = vec![
            TraceSpan {
                trace_id: "trace-1".into(),
                span_id: "span-1".into(),
                parent_span_id: None,
                service_name: "gateway".into(),
                operation_name: "GET /api/health".into(),
                start_time: Utc::now(),
                duration_ms: 5,
                status_code: 0,
                tags: vec![],
            },
            TraceSpan {
                trace_id: "trace-2".into(),
                span_id: "span-2".into(),
                parent_span_id: Some("span-1".into()),
                service_name: "auth".into(),
                operation_name: "ValidateToken".into(),
                start_time: Utc::now(),
                duration_ms: 12,
                status_code: 0,
                tags: vec![],
            },
        ];

        let (processed, metrics) = receiver.process_spans(spans);
        assert_eq!(processed.len(), 2);

        // 应有 gateway 和 auth 各一个计数指标
        let gateway_metric = metrics
            .iter()
            .find(|m| m.labels.iter().any(|(k, v)| k == "service" && v == "gateway"));
        assert!(gateway_metric.is_some());
        assert_eq!(gateway_metric.unwrap().value, 1.0);
    }
}
