//! 告警规则引擎
//!
//! 定义告警规则结构，提供基于阈值/标签匹配的规则评估能力。
//! 支持 >、<、>=、<=、==、!= 六种比较运算符。

use monitor_core::model::alert::AlertLevel;
use monitor_core::model::metric::MetricSample;
use serde::{Deserialize, Serialize};

/// 比较运算符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Gt,     // >
    Lt,     // <
    Gte,    // >=
    Lte,    // <=
    Eq,     // ==
    Neq,    // !=
}

impl CompareOp {
    fn eval(&self, value: f64, threshold: f64) -> bool {
        match self {
            CompareOp::Gt => value > threshold,
            CompareOp::Lt => value < threshold,
            CompareOp::Gte => value >= threshold,
            CompareOp::Lte => value <= threshold,
            CompareOp::Eq => (value - threshold).abs() < 1e-9,
            CompareOp::Neq => (value - threshold).abs() >= 1e-9,
        }
    }
}

/// 标签匹配器：筛选特定标签的指标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelMatcher {
    pub key: String,
    pub value: String,
}

/// 告警规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    /// 规则名称
    pub name: String,
    /// 匹配的指标名称（支持前缀通配，如 `http_*`）
    pub metric_pattern: String,
    /// 标签筛选器（AND 关系）
    #[serde(default)]
    pub label_matchers: Vec<LabelMatcher>,
    /// 比较运算符
    pub op: CompareOp,
    /// 阈值
    pub threshold: f64,
    /// 持续时间（秒），条件持续满足多久后触发
    pub duration_secs: u64,
    /// 告警级别
    pub level: AlertLevel,
    /// 告警摘要
    pub summary: String,
    /// 告警描述
    pub description: String,
    /// 附加到告警的标签
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    /// 告警分组键（用于去重和聚合）
    #[serde(default)]
    pub group_by: Vec<String>,
}

/// 规则评估结果
#[derive(Debug, Clone)]
pub struct TriggerResult {
    /// 规则名称
    pub rule_name: String,
    /// 告警级别
    pub level: AlertLevel,
    /// 匹配到的标签
    pub labels: Vec<(String, String)>,
    /// 当前指标值
    pub current_value: f64,
    /// 摘要（已填充变量）
    pub summary: String,
    /// 描述（已填充变量）
    pub description: String,
    /// 规则要求的持续时间（秒）
    pub duration_secs: u64,
}

/// 规则评估引擎
pub struct RuleEngine {
    rules: Vec<AlertRule>,
}

impl RuleEngine {
    pub fn new(rules: Vec<AlertRule>) -> Self {
        Self { rules }
    }

    /// 添加规则
    pub fn add_rule(&mut self, rule: AlertRule) {
        self.rules.push(rule);
    }

    /// 获取规则数量
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// 评估单个样本是否触发某条规则
    pub fn evaluate_sample(&self, sample: &MetricSample) -> Vec<TriggerResult> {
        let mut results = Vec::new();

        for rule in &self.rules {
            if !metric_name_matches(&rule.metric_pattern, &sample.name) {
                continue;
            }

            if !labels_match(&rule.label_matchers, &sample.labels) {
                continue;
            }

            if !rule.op.eval(sample.value, rule.threshold) {
                continue;
            }

            // 构建告警标签：合并样本标签和规则附加标签
            let mut alert_labels = sample.labels.clone();
            alert_labels.extend(rule.labels.clone());

            let summary = fill_template(&rule.summary, sample, &alert_labels);
            let description = fill_template(&rule.description, sample, &alert_labels);

            results.push(TriggerResult {
                rule_name: rule.name.clone(),
                level: rule.level.clone(),
                labels: alert_labels,
                current_value: sample.value,
                summary,
                description,
                duration_secs: rule.duration_secs,
            });
        }

        results
    }

    /// 批量评估
    pub fn evaluate_batch(&self, samples: &[MetricSample]) -> Vec<TriggerResult> {
        samples
            .iter()
            .flat_map(|s| self.evaluate_sample(s))
            .collect()
    }

    /// 获取规则引用
    pub fn get_rule(&self, name: &str) -> Option<&AlertRule> {
        self.rules.iter().find(|r| r.name == name)
    }
}

/// 指标名匹配（支持 `*` 前缀通配，如 `http_*`）
fn metric_name_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

/// 标签匹配（所有 matcher 必须满足）
fn labels_match(
    matchers: &[LabelMatcher],
    labels: &[(String, String)],
) -> bool {
    if matchers.is_empty() {
        return true;
    }
    matchers.iter().all(|m| {
        labels
            .iter()
            .any(|(k, v)| k == &m.key && v == &m.value)
    })
}

/// 模板变量填充
fn fill_template(
    tmpl: &str,
    sample: &MetricSample,
    labels: &[(String, String)],
) -> String {
    let mut result = tmpl.to_string();
    result = result.replace("$value", &format!("{:.2}", sample.value));
    result = result.replace("$name", &sample.name);

    for (k, v) in labels {
        result = result.replace(&format!("$label_{}", k), v);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample(name: &str, value: f64, labels: Vec<(&str, &str)>) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            value,
            timestamp: chrono::Utc::now(),
        }
    }

    fn make_rule(name: &str, metric: &str, op: CompareOp, threshold: f64, level: AlertLevel) -> AlertRule {
        AlertRule {
            name: name.into(),
            metric_pattern: metric.into(),
            label_matchers: vec![],
            op,
            threshold,
            duration_secs: 60,
            level,
            summary: "$name is $value".into(),
            description: "Rule $name triggered at $value".into(),
            labels: vec![],
            group_by: vec![],
        }
    }

    #[test]
    fn test_compare_ops() {
        assert!(CompareOp::Gt.eval(5.0, 3.0));
        assert!(!CompareOp::Gt.eval(3.0, 5.0));
        assert!(CompareOp::Lt.eval(2.0, 5.0));
        assert!(CompareOp::Gte.eval(5.0, 5.0));
        assert!(CompareOp::Lte.eval(5.0, 5.0));
        assert!(CompareOp::Eq.eval(3.14, 3.14));
        assert!(!CompareOp::Neq.eval(3.14, 3.14));
    }

    #[test]
    fn test_rule_trigger_gt() {
        let engine = RuleEngine::new(vec![
            make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning),
        ]);

        let results = engine.evaluate_sample(&make_sample("cpu_usage", 0.85, vec![]));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_name, "high_cpu");
        assert_eq!(results[0].level, AlertLevel::Warning);
    }

    #[test]
    fn test_rule_not_triggered() {
        let engine = RuleEngine::new(vec![
            make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning),
        ]);

        let results = engine.evaluate_sample(&make_sample("cpu_usage", 0.5, vec![]));
        assert!(results.is_empty());
    }

    #[test]
    fn test_label_matcher() {
        let rule = AlertRule {
            label_matchers: vec![LabelMatcher {
                key: "service".into(),
                value: "gateway".into(),
            }],
            ..make_rule("gateway_error", "http_errors", CompareOp::Gt, 100.0, AlertLevel::Critical)
        };

        let engine = RuleEngine::new(vec![rule]);

        // 匹配
        let r1 = engine.evaluate_sample(&make_sample("http_errors", 150.0, vec![("service", "gateway")]));
        assert_eq!(r1.len(), 1);

        // 不匹配（标签不对）
        let r2 = engine.evaluate_sample(&make_sample("http_errors", 150.0, vec![("service", "auth")]));
        assert!(r2.is_empty());
    }

    #[test]
    fn test_metric_pattern_wildcard() {
        let rule = AlertRule {
            metric_pattern: "http_*".into(),
            ..make_rule("http_errors", "", CompareOp::Gt, 100.0, AlertLevel::Warning)
        };

        let engine = RuleEngine::new(vec![rule]);

        assert_eq!(engine.evaluate_sample(&make_sample("http_requests", 200.0, vec![])).len(), 1);
        assert_eq!(engine.evaluate_sample(&make_sample("http_errors", 150.0, vec![])).len(), 1);
        assert_eq!(engine.evaluate_sample(&make_sample("cpu_usage", 200.0, vec![])).len(), 0);
    }

    #[test]
    fn test_template_filling() {
        let rule = AlertRule {
            summary: "Service $label_service: $name = $value".into(),
            description: "Value $value exceeds threshold".into(),
            ..make_rule("test", "metric", CompareOp::Gt, 0.0, AlertLevel::Info)
        };

        let engine = RuleEngine::new(vec![rule]);
        let results = engine.evaluate_sample(&make_sample(
            "metric", 42.5,
            vec![("service", "gateway")],
        ));

        assert_eq!(results[0].summary, "Service gateway: metric = 42.50");
        assert_eq!(results[0].description, "Value 42.50 exceeds threshold");
    }

    #[test]
    fn test_batch_evaluation() {
        let engine = RuleEngine::new(vec![
            make_rule("high_cpu", "cpu_usage", CompareOp::Gt, 0.8, AlertLevel::Warning),
            make_rule("high_mem", "mem_usage", CompareOp::Gt, 0.9, AlertLevel::Critical),
        ]);

        let samples = vec![
            make_sample("cpu_usage", 0.85, vec![]),
            make_sample("mem_usage", 0.95, vec![]),
            make_sample("cpu_usage", 0.5, vec![]), // 不触发
        ];

        let results = engine.evaluate_batch(&samples);
        assert_eq!(results.len(), 2);
    }
}
