//! 流式处理引擎
//!
//! 通过 Tokio channel 接收指标样本流，
//! 按时间窗口聚合后输出聚合指标。
//! 支持可配置的窗口刷新间隔和背压控制。

use super::aggregator::{AggKind, AggregatedMetric, Aggregator, AggregatorConfig};
use monitor_core::model::metric::MetricSample;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// 流处理器配置
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// 聚合器配置
    pub aggregator: AggregatorConfig,
    /// 输入 channel 缓冲区大小
    pub input_buffer: usize,
    /// 输出 channel 缓冲区大小
    pub output_buffer: usize,
    /// 窗口刷新检查间隔（秒），用于定期 flush 过期窗口
    pub flush_interval_secs: u64,
    /// 窗口最大存活时间（秒），超过此时间的窗口强制 flush
    pub max_window_age_secs: i64,
    /// 输出的聚合函数列表
    pub output_kinds: Vec<AggKind>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            aggregator: AggregatorConfig::default(),
            input_buffer: 4096,
            output_buffer: 1024,
            flush_interval_secs: 30,
            max_window_age_secs: 300,
            output_kinds: vec![AggKind::Avg, AggKind::Max],
        }
    }
}

/// 流处理器
pub struct StreamProcessor {
    config: StreamConfig,
}

impl StreamProcessor {
    pub fn new(config: StreamConfig) -> Self {
        Self { config }
    }

    /// 启动流处理，返回输入/输出 channel 端点
    pub fn channels(&self) -> (mpsc::Sender<MetricSample>, mpsc::Receiver<AggregatedMetric>) {
        let (input_tx, input_rx) = mpsc::channel(self.config.input_buffer);
        let (output_tx, output_rx) = mpsc::channel(self.config.output_buffer);

        let config = self.config.clone();
        tokio::spawn(async move {
            run_loop(config, input_rx, output_tx).await;
        });

        (input_tx, output_rx)
    }
}

/// 主处理循环
async fn run_loop(
    config: StreamConfig,
    mut input_rx: mpsc::Receiver<MetricSample>,
    output_tx: mpsc::Sender<AggregatedMetric>,
) {
    let window_secs = config.aggregator.window_secs;
    let flush_interval = config.flush_interval_secs;
    let mut aggregator = Aggregator::new(config.aggregator);
    let mut flush_timer = tokio::time::interval(
        std::time::Duration::from_secs(flush_interval),
    );

    info!(
        window_secs,
        flush_interval,
        "stream processor started"
    );

    loop {
        tokio::select! {
            // 接收样本
            maybe_sample = input_rx.recv() => {
                match maybe_sample {
                    Some(sample) => {
                        let flushed = aggregator.push(sample);
                        for metric in flushed {
                            if output_tx.send(metric).await.is_err() {
                                warn!("output channel closed, stopping processor");
                                return;
                            }
                        }
                    }
                    None => {
                        // 输入 channel 关闭，flush 剩余窗口
                        info!("input channel closed, flushing remaining windows");
                        flush_all(&mut aggregator, &output_tx).await;
                        return;
                    }
                }
            }

            // 定期强制 flush 过期窗口
            _ = flush_timer.tick() => {
                debug!("periodic flush tick");
                flush_expired(&mut aggregator, &output_tx, config.max_window_age_secs).await;
            }
        }
    }
}

/// 刷新所有窗口
async fn flush_all(aggregator: &mut Aggregator, output: &mpsc::Sender<AggregatedMetric>) {
    if let Some(window_start) = aggregator.current_window() {
        let metrics = aggregator.flush_window(window_start);
        for metric in metrics {
            let _ = output.send(metric).await;
        }
    }
}

/// 刷新过期窗口（根据 max_window_age_secs 判断）
async fn flush_expired(
    aggregator: &mut Aggregator,
    output: &mpsc::Sender<AggregatedMetric>,
    max_age_secs: i64,
) {
    let now = chrono::Utc::now().timestamp();
    // 简单实现：如果当前窗口超过最大存活时间，flush 它
    if let Some(ws) = aggregator.current_window() {
        if now - ws > max_age_secs {
            let metrics = aggregator.flush_window(ws);
            for metric in metrics {
                let _ = output.send(metric).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_sample(name: &str, value: f64, ts_secs: i64) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: Vec::new(),
            value,
            timestamp: Utc.timestamp_opt(ts_secs, 0).unwrap(),
        }
    }

    #[tokio::test]
    async fn test_stream_basic() {
        let config = StreamConfig {
            aggregator: AggregatorConfig {
                window_secs: 60,
                alignment_secs: 0,
            },
            flush_interval_secs: 1,
            max_window_age_secs: 10,
            ..Default::default()
        };

        let processor = StreamProcessor::new(config);
        let (tx, mut rx) = processor.channels();

        // 发送同一窗口的样本
        tx.send(make_sample("cpu", 0.5, 1001)).await.unwrap();
        tx.send(make_sample("cpu", 0.7, 1002)).await.unwrap();
        tx.send(make_sample("cpu", 0.6, 1003)).await.unwrap();

        // 发送下一窗口的样本，触发刷新
        tx.send(make_sample("cpu", 1.0, 1070)).await.unwrap();

        // 接收刷新后的聚合指标
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(result.name, "cpu");
        // avg of (0.5, 0.7, 0.6) = 0.6
        assert!((result.value - 0.6).abs() < 0.001);
        assert_eq!(result.sample_count, 3);
    }

    #[tokio::test]
    async fn test_stream_channel_close() {
        let config = StreamConfig {
            aggregator: AggregatorConfig::default(),
            flush_interval_secs: 1,
            max_window_age_secs: 10,
            ..Default::default()
        };

        let processor = StreamProcessor::new(config);
        let (tx, mut rx) = processor.channels();

        tx.send(make_sample("cpu", 0.5, 0)).await.unwrap();
        drop(tx); // 关闭输入

        // 应该能收到 flush 的结果
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(result.name, "cpu");
        assert_eq!(result.value, 0.5);
    }
}
