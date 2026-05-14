//! Prometheus 指标抓取器
//!
//! 包含 Prometheus exposition format 解析器和 HTTP scraper。

pub mod parser;

use monitor_core::config::ScrapeTarget;
use monitor_core::model::metric::MetricSample;
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, warn};

/// 从单个目标抓取指标
///
/// 发起 HTTP GET 请求到目标的 /metrics 端点，
/// 解析响应文本为 [MetricSample] 列表，并注入目标配置的业务标签。
pub async fn scrape_target(
    client: &Client,
    target: &ScrapeTarget,
    timeout_secs: u64,
) -> anyhow::Result<Vec<MetricSample>> {
    let resp = client
        .get(&target.url)
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await?;

    if !resp.status().is_success() {
        warn!(
            target = target.name,
            status = resp.status().as_u16(),
            "scrape returned non-200"
        );
        return Ok(Vec::new());
    }

    let body = resp.text().await?;
    let mut samples = parser::parse_text(&body);

    // 注入目标配置的业务标签
    if !target.labels.is_empty() {
        for sample in &mut samples {
            sample.labels.extend(target.labels.clone());
        }
    }

    debug!(
        target = target.name,
        count = samples.len(),
        "scrape complete"
    );

    Ok(samples)
}

/// 根据目标列表并发抓取所有目标
pub async fn scrape_all(
    client: &Client,
    targets: &[ScrapeTarget],
    timeout_secs: u64,
) -> Vec<(String, Vec<MetricSample>)> {
    let futures: Vec<_> = targets
        .iter()
        .map(|t| {
            let name = t.name.clone();
            let fut = scrape_target(client, t, timeout_secs);
            async move { (name, fut.await) }
        })
        .collect();

    let mut results = Vec::new();
    for fut in futures {
        let (name, result) = fut.await;
        match result {
            Ok(samples) => results.push((name, samples)),
            Err(e) => {
                warn!(target = name, error = %e, "scrape failed");
            }
        }
    }

    results
}
