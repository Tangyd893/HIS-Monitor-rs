//! 通知通道实现
//!
//! - ConsoleChannel: 调试用，输出到 stdout
//! - WebhookChannel: 通用 Webhook 发送器（钉钉/企微）

use super::NotifyChannel;
use crate::router::Channel;
use monitor_core::model::alert::Alert;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, info};

// ── Console 调试通道 ──

/// 控制台调试通道
///
/// 将所有告警通知输出到控制台，用于开发调试。
pub struct ConsoleChannel;

impl ConsoleChannel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl NotifyChannel for ConsoleChannel {
    fn channel_type(&self) -> Channel {
        Channel::Email
    }

    fn name(&self) -> &str {
        "console"
    }

    async fn send(&self, alert: &Alert, receivers: &[String]) -> anyhow::Result<()> {
        let msg = json!({
            "alert_id": alert.id,
            "rule": alert.rule_name,
            "level": format!("{:?}", alert.level),
            "status": format!("{:?}", alert.status),
            "summary": alert.summary,
            "description": alert.description,
            "service": alert.service_name,
            "current_value": alert.current_value,
            "receivers": receivers,
        });

        info!(
            target: "notify.console",
            "{}",
            serde_json::to_string_pretty(&msg).unwrap_or_default()
        );
        Ok(())
    }
}

// ── Webhook 通用通道 ──

/// Webhook 通知通道
///
/// 支持钉钉、企业微信等 Webhook 机器人。
pub struct WebhookChannel {
    channel_type: Channel,
    webhook_url: String,
    secret: Option<String>,
    client: Client,
}

impl WebhookChannel {
    /// 创建 Webhook 通道
    pub fn new(channel_type: Channel, webhook_url: String, secret: Option<String>) -> Self {
        Self {
            channel_type,
            webhook_url,
            secret,
            client: Client::new(),
        }
    }

    /// 钉钉 Webhook 通道
    pub fn dingtalk(webhook_url: String, secret: Option<String>) -> Self {
        Self::new(Channel::DingTalk, webhook_url, secret)
    }

    /// 企业微信 Webhook 通道
    pub fn wecom(webhook_url: String) -> Self {
        Self::new(Channel::WeCom, webhook_url, None)
    }

    /// 构建消息体
    fn build_message(&self, alert: &Alert, _receivers: &[String]) -> serde_json::Value {
        match self.channel_type {
            Channel::DingTalk => self.build_dingtalk_message(alert),
            Channel::WeCom => self.build_wecom_message(alert),
            _ => json!({
                "text": format!(
                    "[{}] {} - {}",
                    format!("{:?}", alert.level),
                    alert.rule_name,
                    alert.summary
                )
            }),
        }
    }

    /// 钉钉 Markdown 消息格式
    fn build_dingtalk_message(&self, alert: &Alert) -> serde_json::Value {
        let level_emoji = match alert.level {
            monitor_core::model::alert::AlertLevel::Emergency => "🔴🔴",
            monitor_core::model::alert::AlertLevel::Critical => "🔴",
            monitor_core::model::alert::AlertLevel::Warning => "🟡",
            monitor_core::model::alert::AlertLevel::Info => "🔵",
        };

        let title = format!(
            "{} [{}] {}",
            level_emoji,
            format!("{:?}", alert.level),
            alert.summary
        );

        let text = format!(
            "## {}\n\n\
             **规则名称**: {}\n\n\
             **服务**: {}\n\n\
             **描述**: {}\n\n\
             **当前值**: {:?}\n\n\
             **触发时间**: {}",
            title,
            alert.rule_name,
            alert.service_name,
            alert.description,
            alert.current_value,
            alert.fired_at.format("%Y-%m-%d %H:%M:%S"),
        );

        json!({
            "msgtype": "markdown",
            "markdown": {
                "title": title,
                "text": text
            }
        })
    }

    /// 企业微信 Markdown 消息格式
    fn build_wecom_message(&self, alert: &Alert) -> serde_json::Value {
        let level_color = match alert.level {
            monitor_core::model::alert::AlertLevel::Emergency => "warning",
            monitor_core::model::alert::AlertLevel::Critical => "warning",
            monitor_core::model::alert::AlertLevel::Warning => "comment",
            monitor_core::model::alert::AlertLevel::Info => "info",
        };

        let content = format!(
            "**<font color=\"{}\">[{}] {}</font>**\n\
             >规则: {}\n\
             >服务: {}\n\
             >描述: {}\n\
             >当前值: {:?}\n\
             >触发时间: {}",
            level_color,
            format!("{:?}", alert.level),
            alert.summary,
            alert.rule_name,
            alert.service_name,
            alert.description,
            alert.current_value,
            alert.fired_at.format("%Y-%m-%d %H:%M:%S"),
        );

        json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content
            }
        })
    }
}

#[async_trait::async_trait]
impl NotifyChannel for WebhookChannel {
    fn channel_type(&self) -> Channel {
        self.channel_type.clone()
    }

    fn name(&self) -> &str {
        match self.channel_type {
            Channel::DingTalk => "dingtalk-webhook",
            Channel::WeCom => "wecom-webhook",
            Channel::Email => "email-webhook",
            Channel::Sms => "sms-webhook",
            Channel::Phone => "phone-webhook",
        }
    }

    async fn send(&self, alert: &Alert, receivers: &[String]) -> anyhow::Result<()> {
        let body = self.build_message(alert, receivers);

        debug!(
            target = self.name(),
            alert_id = alert.id,
            "sending webhook to {}",
            self.webhook_url
        );

        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let resp_body = resp.text().await.unwrap_or_default();

        if status.is_success() {
            debug!(
                target = self.name(),
                alert_id = alert.id,
                status = status.as_u16(),
                "webhook sent"
            );
        } else {
            return Err(anyhow::anyhow!(
                "webhook failed: status={}, body={}",
                status,
                resp_body
            ));
        }

        Ok(())
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use monitor_core::model::alert::{Alert, AlertLevel, AlertStatus};

    fn make_alert(level: AlertLevel) -> Alert {
        Alert {
            id: "test-1".into(),
            rule_name: "high_cpu".into(),
            level,
            status: AlertStatus::Firing,
            summary: "CPU usage high".into(),
            description: "CPU usage exceeded 80%".into(),
            service_name: "gateway".into(),
            fired_at: Utc::now(),
            acked_at: None,
            resolved_at: None,
            silenced_until: None,
            current_value: Some(85.5),
            labels: vec![("service".into(), "gateway".into())],
            group_by: vec![],
        }
    }

    #[tokio::test]
    async fn test_console_channel() {
        let chan = ConsoleChannel::new();
        let alert = make_alert(AlertLevel::Warning);
        let result = chan.send(&alert, &["test".into()]).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_dingtalk_message_format() {
        let chan = WebhookChannel::dingtalk("https://example.com/webhook".into(), None);
        let alert = make_alert(AlertLevel::Critical);
        let msg = chan.build_dingtalk_message(&alert);

        assert_eq!(msg["msgtype"], "markdown");
        assert!(msg["markdown"]["title"].as_str().unwrap().contains("Critical"));
        assert!(msg["markdown"]["text"].as_str().unwrap().contains("high_cpu"));
    }

    #[test]
    fn test_wecom_message_format() {
        let chan = WebhookChannel::wecom("https://example.com/webhook".into());
        let alert = make_alert(AlertLevel::Emergency);
        let msg = chan.build_wecom_message(&alert);

        assert_eq!(msg["msgtype"], "markdown");
        assert!(msg["markdown"]["content"].as_str().unwrap().contains("Emergency"));
        assert!(msg["markdown"]["content"].as_str().unwrap().contains("high_cpu"));
    }
}
