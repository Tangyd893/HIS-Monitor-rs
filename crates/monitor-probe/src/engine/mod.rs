//! 探活引擎
//!
//! 基于场景编排的探活引擎，支持逐步 HTTP 请求 + 上下文传递。
//! 每个场景包含多个步骤，步骤间通过 JSONPath 提取变量传递。

use crate::scenario::ProbeScenario;
use chrono::Utc;
use monitor_core::model::metric::MetricSample;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, error, info, warn};

/// 单个探活步骤
#[derive(Debug, Clone)]
pub struct ProbeStep {
    pub name: String,
    pub method: String,
    pub url_template: String,
    pub body: Option<String>,
    pub expect_status: u16,
    pub extract: Vec<(String, String)>,
}

/// 步骤执行结果
#[derive(Debug, Clone)]
pub struct StepResult {
    pub step_name: String,
    pub status: u16,
    pub latency_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// 探活引擎
pub struct ProbeEngine {
    client: Client,
    base_url: String,
}

impl ProbeEngine {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            base_url,
        }
    }

    fn render_url(&self, template: &str, vars: &HashMap<String, String>) -> String {
        let mut url = template.to_string();
        if !url.starts_with("http") {
            url = format!("{}{}", self.base_url.trim_end_matches('/'), url);
        }
        for (key, val) in vars {
            url = url.replace(&format!("{{{{{}}}}}", key), val);
        }
        url
    }

    fn render_body(body: &str, vars: &HashMap<String, String>) -> String {
        let mut result = body.to_string();
        for (key, val) in vars {
            result = result.replace(&format!("{{{{{}}}}}", key), val);
        }
        result
    }

    async fn execute_step(
        &self,
        step: &ProbeStep,
        vars: &HashMap<String, String>,
    ) -> (StepResult, HashMap<String, String>) {
        let url = self.render_url(&step.url_template, vars);
        let start = Instant::now();

        let result = match step.method.to_uppercase().as_str() {
            "GET" => self.client.get(&url).send().await,
            "POST" => {
                let mut req = self.client.post(&url);
                if let Some(body) = &step.body {
                    let rendered = Self::render_body(body, vars);
                    req = req.header("Content-Type", "application/json").body(rendered);
                }
                req.send().await
            }
            _ => {
                return (
                    StepResult {
                        step_name: step.name.clone(),
                        status: 0,
                        latency_ms: start.elapsed().as_millis() as u64,
                        success: false,
                        error: Some(format!("unsupported method: {}", step.method)),
                    },
                    vars.clone(),
                );
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let success = status == step.expect_status;

                let mut new_vars = vars.clone();
                if success && !step.extract.is_empty() {
                    if let Ok(body) = resp.text().await {
                        if let Ok(json) = serde_json::from_str::<Value>(&body) {
                            for (path, var_name) in &step.extract {
                                if let Some(val) = extract_json_value(&json, path) {
                                    new_vars.insert(var_name.clone(), val);
                                }
                            }
                        }
                    }
                }

                if !success {
                    warn!(
                        step = step.name,
                        expected = step.expect_status,
                        actual = status,
                        latency_ms,
                        "step failed"
                    );
                }

                (
                    StepResult {
                        step_name: step.name.clone(),
                        status,
                        latency_ms,
                        success,
                        error: if success {
                            None
                        } else {
                            Some(format!("expected {}, got {}", step.expect_status, status))
                        },
                    },
                    new_vars,
                )
            }
            Err(e) => {
                error!(step = step.name, error = %e, latency_ms, "step error");
                (
                    StepResult {
                        step_name: step.name.clone(),
                        status: 0,
                        latency_ms,
                        success: false,
                        error: Some(e.to_string()),
                    },
                    vars.clone(),
                )
            }
        }
    }

    /// 执行完整场景
    pub async fn execute_scenario(
        &self,
        scenario: &ProbeScenario,
    ) -> (Vec<StepResult>, Vec<MetricSample>) {
        info!(scenario = scenario.name, "executing scenario");
        let mut vars = scenario.init_vars.clone();
        let mut results = Vec::new();
        let mut success = true;

        for step in &scenario.steps {
            let (result, new_vars) = self.execute_step(step, &vars).await;
            vars = new_vars;
            success = success && result.success;
            results.push(result);

            if !success && !scenario.continue_on_error {
                warn!(scenario = scenario.name, "stopping on error");
                break;
            }
        }

        let now = Utc::now();
        let base_labels = vec![
            ("scenario".into(), scenario.name.clone()),
            ("type".into(), "probe".into()),
        ];

        // 按步指标
        let mut metrics: Vec<MetricSample> = results
            .iter()
            .map(|r| {
                let mut labels = base_labels.clone();
                labels.push(("step".into(), r.step_name.clone()));
                MetricSample {
                    name: "his_probe_step_duration_ms".into(),
                    labels,
                    value: r.latency_ms as f64,
                    timestamp: now,
                }
            })
            .collect();

        // 整体耗时指标
        let mut overall_labels = base_labels.clone();
        overall_labels.push(("step".into(), "overall".into()));
        let total_latency: u64 = results.iter().map(|r| r.latency_ms).sum();
        metrics.push(MetricSample {
            name: "his_probe_scenario_duration_ms".into(),
            labels: overall_labels,
            value: total_latency as f64,
            timestamp: now,
        });

        // 成功状态指标
        metrics.push(MetricSample {
            name: "his_probe_success".into(),
            labels: base_labels,
            value: if success { 1.0 } else { 0.0 },
            timestamp: now,
        });

        info!(
            scenario = scenario.name,
            success,
            steps = results.len(),
            "scenario completed"
        );

        (results, metrics)
    }
}

/// 从 JSON 值中提取简单路径字段
fn extract_json_value(json: &Value, path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = json;

    for seg in &segments {
        if let Some(bracket_pos) = seg.find('[') {
            let field = &seg[..bracket_pos];
            let rest = &seg[bracket_pos..];

            if !field.is_empty() {
                current = current.get(field)?;
            }

            for part in rest.split(']') {
                let part = part.trim_start_matches('[');
                if let Ok(idx) = part.parse::<usize>() {
                    current = current.get(idx)?;
                }
            }
        } else {
            current = current.get(seg)?;
        }
    }

    match current {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(current.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_value() {
        let json: Value = serde_json::from_str(
            r#"{"data": {"token": "abc123", "user": {"name": "test"}}}"#,
        )
        .unwrap();

        assert_eq!(
            extract_json_value(&json, "data.token"),
            Some("abc123".into())
        );
        assert_eq!(
            extract_json_value(&json, "data.user.name"),
            Some("test".into())
        );
        assert_eq!(extract_json_value(&json, "data.missing"), None);
    }

    #[test]
    fn test_render_url() {
        let engine = ProbeEngine::new("http://gateway:8080".into());
        let mut vars = HashMap::new();
        vars.insert("token".into(), "abc123".into());
        vars.insert("id".into(), "42".into());

        assert_eq!(
            engine.render_url("/api/order/{{id}}", &vars),
            "http://gateway:8080/api/order/42"
        );
        assert_eq!(
            engine.render_url("http://other:9090/login", &vars),
            "http://other:9090/login"
        );
    }
}
