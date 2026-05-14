//! Prometheus 文本展示格式解析器
//!
//! 解析 Prometheus exposition format，将文本行转换为 [MetricSample] 集合。
//!
//! 支持的格式：
//! - Counter / Gauge / Histogram / Summary / Untyped
//! - 带标签：`metric_name{label="value"} 1.0 1620000000`
//! - 不带标签：`metric_name 1.0`
//! - HELP / TYPE 注释行

use monitor_core::model::metric::MetricSample;
use chrono::Utc;

/// 解析一行 Prometheus 指标文本
///
/// 返回 `None` 表示该行不包含指标数据（注释行、空行、HELP/TYPE）。
pub fn parse_line(line: &str) -> Option<MetricSample> {
    let line = line.trim();

    // 跳过空行和注释行
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // 按空格分割所有 token
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    // 第一个 token 是指标名（可能带标签）
    let name_labels = tokens[0];
    let (name, labels) = parse_name_and_labels(name_labels)?;

    // 第二个 token 是值
    let value = parse_value(tokens[1])?;

    // 第三个 token（如果有）是时间戳，忽略因为我们用当前时间

    Some(MetricSample {
        name,
        labels,
        value,
        timestamp: Utc::now(),
    })
}

/// 将完整的 Prometheus 指标文本解析为样本列表
pub fn parse_text(text: &str) -> Vec<MetricSample> {
    text.lines()
        .filter_map(parse_line)
        .collect()
}

/// 解析 `metric_name{key="val",...}` 或 `metric_name`
fn parse_name_and_labels(s: &str) -> Option<(String, Vec<(String, String)>)> {
    if let Some(brace_pos) = s.find('{') {
        let name = s[..brace_pos].to_string();
        let labels_str = &s[brace_pos + 1..];

        // 找到匹配的 }
        let close_pos = labels_str.rfind('}')?;
        let labels_str = &labels_str[..close_pos];

        let labels = parse_label_pairs(labels_str).unwrap_or_default();
        Some((name, labels))
    } else {
        Some((s.to_string(), Vec::new()))
    }
}

/// 解析标签对 `key="value",key2="value2"`
fn parse_label_pairs(s: &str) -> Option<Vec<(String, String)>> {
    let mut labels = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // 跳过空白和逗号
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        // 读取 key
        let key_start = i;
        while i < chars.len() && chars[i] != '=' {
            i += 1;
        }
        let key = chars[key_start..i].iter().collect::<String>();

        if i >= chars.len() || chars[i] != '=' {
            return None;
        }
        i += 1; // 跳过 =

        // 读取 value（引号包围）
        if i >= chars.len() || chars[i] != '"' {
            return None;
        }
        i += 1;
        let value = read_quoted_value(&chars, &mut i);
        // 这里的 i 已经指向结束引号之后

        labels.push((key, value));
    }

    Some(labels)
}

/// 读取引号内的值，处理转义 `\\` → `\`, `\"` → `"`
fn read_quoted_value(chars: &[char], i: &mut usize) -> String {
    let mut result = String::new();

    while *i < chars.len() {
        if chars[*i] == '\\' {
            *i += 1;
            if *i < chars.len() {
                match chars[*i] {
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    'n' => result.push('\n'),
                    other => {
                        result.push('\\');
                        result.push(other);
                    }
                }
                *i += 1;
            }
        } else if chars[*i] == '"' {
            *i += 1; // 跳过结束引号
            break;
        } else {
            result.push(chars[*i]);
            *i += 1;
        }
    }

    result
}

/// 解析值
fn parse_value(s: &str) -> Option<f64> {
    match s {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        "NaN" => Some(f64::NAN),
        other => other.parse::<f64>().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_gauge() {
        let sample = parse_line("http_requests_total 1027").unwrap();
        assert_eq!(sample.name, "http_requests_total");
        assert_eq!(sample.value, 1027.0);
        assert!(sample.labels.is_empty());
    }

    #[test]
    fn test_parse_with_labels() {
        let sample = parse_line(
            r#"http_requests_total{method="POST",handler="/api/v1"} 1027"#
        ).unwrap();
        assert_eq!(sample.name, "http_requests_total");
        assert_eq!(sample.value, 1027.0);
        assert_eq!(sample.labels.len(), 2);
        assert_eq!(sample.labels[0], ("method".into(), "POST".into()));
        assert_eq!(sample.labels[1], ("handler".into(), "/api/v1".into()));
    }

    #[test]
    fn test_parse_with_timestamp() {
        let sample = parse_line("cpu_usage 0.85 1620000000").unwrap();
        assert_eq!(sample.value, 0.85);
    }

    #[test]
    fn test_parse_special_values() {
        assert!(parse_line("metric +Inf").unwrap().value.is_infinite());
        assert!(parse_line("metric -Inf").unwrap().value.is_infinite());
        assert!(parse_line("metric NaN").unwrap().value.is_nan());
    }

    #[test]
    fn test_skip_comments() {
        assert!(parse_line("# HELP metric description").is_none());
        assert!(parse_line("# TYPE metric counter").is_none());
        assert!(parse_line("").is_none());
        assert!(parse_line("  # commented").is_none());
    }

    #[test]
    fn test_parse_text() {
        let text = r#"# HELP http_requests_total Total HTTP requests
# TYPE http_requests_total counter
http_requests_total{method="GET"} 500
http_requests_total{method="POST"} 527
# TYPE memory_bytes gauge
memory_bytes 1.048576e+06
"#;
        let samples = parse_text(text);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].name, "http_requests_total");
        assert_eq!(samples[2].name, "memory_bytes");
        assert_eq!(samples[2].value, 1_048_576.0);
    }

    #[test]
    fn test_parse_label_with_escape() {
        let sample = parse_line(r#"metric{path="a\\b\"c"} 1"#).unwrap();
        assert_eq!(sample.labels[0].1, r#"a\b"c"#);
    }

    #[test]
    fn test_his_probe_metrics() {
        // 模拟 HIS-Go 探活指标格式
        let text = r#"# HELP his_probe_success HIS probe success status
# TYPE his_probe_success gauge
his_probe_success{chain="registration",step="submit"} 1
his_probe_success{chain="registration",step="verify"} 1
his_probe_duration_seconds{chain="prescription",step="audit"} 0.452
"#;
        let samples = parse_text(text);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].name, "his_probe_success");
        assert_eq!(samples[0].labels[0], ("chain".into(), "registration".into()));
        assert_eq!(samples[0].labels[1], ("step".into(), "submit".into()));
    }
}
