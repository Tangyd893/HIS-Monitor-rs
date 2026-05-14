//! 静默窗口管理器
//!
//! 支持按标签匹配的告警静默，用于计划维护期间抑制告警通知。
//! 静默规则可通过 API 动态创建/删除，也支持预定义的维护窗口。

use monitor_core::model::alert::Alert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};
use uuid::Uuid;

// ── 静默规则 ──

/// 标签匹配器（与 rule 模块保持一致）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SilenceLabelMatcher {
    pub key: String,
    pub value: String,
}

/// 静默规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceRule {
    /// 静默规则唯一 ID
    pub id: String,
    /// 创建者（用户或系统）
    #[serde(default)]
    pub created_by: String,
    /// 静默原因
    #[serde(default)]
    pub reason: String,
    /// 静默开始时间
    pub starts_at: DateTime<Utc>,
    /// 静默结束时间
    pub ends_at: DateTime<Utc>,
    /// 标签匹配器列表（AND 关系），为空则匹配所有告警
    #[serde(default)]
    pub matchers: Vec<SilenceLabelMatcher>,
    /// 是否只静默特定级别及以上的告警
    #[serde(default)]
    pub min_severity: Option<monitor_core::model::alert::AlertLevel>,
}

impl SilenceRule {
    /// 判断当前时刻是否在静默窗口内
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        now >= self.starts_at && now < self.ends_at
    }

    /// 判断给定告警是否匹配此静默规则
    pub fn matches_alert(&self, alert: &Alert) -> bool {
        // 检查级别
        if let Some(ref min_level) = self.min_severity {
            if alert.level < *min_level {
                return false;
            }
        }

        // 检查标签匹配
        if self.matchers.is_empty() {
            return true;
        }

        self.matchers.iter().all(|m| {
            alert
                .labels
                .iter()
                .any(|(k, v)| k == &m.key && v == &m.value)
        })
    }
}

// ── 静默管理器 ──

/// 静默窗口管理器
pub struct SilenceManager {
    /// 活跃的静默规则（按 ID 索引）
    rules: HashMap<String, SilenceRule>,
    /// 已过期但保留的规则（用于审计追溯）
    expired: Vec<SilenceRule>,
}

impl SilenceManager {
    /// 创建空的静默管理器
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
            expired: Vec::new(),
        }
    }

    /// 添加静默规则
    pub fn add(&mut self, mut rule: SilenceRule) -> String {
        if rule.id.is_empty() {
            rule.id = Uuid::new_v4().to_string();
        }
        let id = rule.id.clone();
        debug!(
            id = id,
            starts = %rule.starts_at,
            ends = %rule.ends_at,
            matchers = rule.matchers.len(),
            "silence rule added"
        );
        self.rules.insert(id.clone(), rule);
        id
    }

    /// 创建计划维护静默
    pub fn add_maintenance(
        &mut self,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        reason: &str,
        service_matchers: Vec<(&str, &str)>,
    ) -> String {
        let matchers: Vec<SilenceLabelMatcher> = service_matchers
            .into_iter()
            .map(|(k, v)| SilenceLabelMatcher {
                key: k.to_string(),
                value: v.to_string(),
            })
            .collect();

        let rule = SilenceRule {
            id: Uuid::new_v4().to_string(),
            created_by: "system".into(),
            reason: reason.to_string(),
            starts_at,
            ends_at,
            matchers,
            min_severity: None,
        };

        info!(
            id = rule.id,
            reason = rule.reason,
            "maintenance silence created"
        );
        self.add(rule)
    }

    /// 移除静默规则
    pub fn remove(&mut self, id: &str) -> Option<SilenceRule> {
        let rule = self.rules.remove(id);
        if let Some(ref r) = rule {
            debug!(id = r.id, "silence rule removed");
        }
        rule
    }

    /// 判断告警是否被静默
    ///
    /// 返回匹配的静默规则（如果存在）。
    pub fn is_silenced(&self, alert: &Alert) -> Option<&SilenceRule> {
        let now = Utc::now();
        self.rules
            .values()
            .find(|rule| rule.is_active(now) && rule.matches_alert(alert))
    }

    /// 获取所有活跃的静默规则
    pub fn active_rules(&self) -> Vec<&SilenceRule> {
        let now = Utc::now();
        self.rules
            .values()
            .filter(|r| r.is_active(now))
            .collect()
    }

    /// 获取所有规则（含未来的）
    pub fn all_rules(&self) -> Vec<&SilenceRule> {
        self.rules.values().collect()
    }

    /// 清除过期的静默规则（移至 expired 列表）
    pub fn tick_expired(&mut self) -> usize {
        let now = Utc::now();
        let expired_ids: Vec<String> = self
            .rules
            .iter()
            .filter(|(_, rule)| rule.ends_at <= now)
            .map(|(id, _)| id.clone())
            .collect();

        let count = expired_ids.len();
        for id in &expired_ids {
            if let Some(rule) = self.rules.remove(id) {
                debug!(id = rule.id, "silence rule expired");
                self.expired.push(rule);
            }
        }

        if count > 0 {
            info!(count, "cleaned expired silence rules");
        }

        count
    }

    /// 获取已过期规则数量
    pub fn expired_count(&self) -> usize {
        self.expired.len()
    }
}

impl Default for SilenceManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use monitor_core::model::alert::{Alert, AlertLevel, AlertStatus};

    fn make_alert(service: &str, level: AlertLevel) -> Alert {
        Alert {
            id: Uuid::new_v4().to_string(),
            rule_name: "test_rule".into(),
            level,
            status: AlertStatus::Firing,
            summary: "test".into(),
            description: "test description".into(),
            service_name: service.to_string(),
            fired_at: Utc::now(),
            acked_at: None,
            resolved_at: None,
            silenced_until: None,
            current_value: Some(1.0),
            labels: vec![("service".into(), service.to_string())],
            group_by: vec![],
        }
    }

    #[test]
    fn test_rule_active_window() {
        let now = Utc::now();
        let rule = SilenceRule {
            id: "s1".into(),
            created_by: "test".into(),
            reason: "test".into(),
            starts_at: now - chrono::Duration::minutes(5),
            ends_at: now + chrono::Duration::minutes(5),
            matchers: vec![],
            min_severity: None,
        };

        assert!(rule.is_active(now));
        assert!(!rule.is_active(now - chrono::Duration::minutes(10)));
        assert!(!rule.is_active(now + chrono::Duration::minutes(10)));
    }

    #[test]
    fn test_rule_matches_by_label() {
        let now = Utc::now();
        let rule = SilenceRule {
            id: "s2".into(),
            created_by: "test".into(),
            reason: "test".into(),
            starts_at: now - chrono::Duration::minutes(5),
            ends_at: now + chrono::Duration::minutes(5),
            matchers: vec![SilenceLabelMatcher {
                key: "service".into(),
                value: "gateway".into(),
            }],
            min_severity: None,
        };

        let alert_gateway = make_alert("gateway", AlertLevel::Warning);
        let alert_auth = make_alert("auth", AlertLevel::Warning);

        assert!(rule.matches_alert(&alert_gateway));
        assert!(!rule.matches_alert(&alert_auth));
    }

    #[test]
    fn test_rule_matches_by_severity() {
        let now = Utc::now();
        let rule = SilenceRule {
            id: "s3".into(),
            created_by: "test".into(),
            reason: "test".into(),
            starts_at: now - chrono::Duration::minutes(5),
            ends_at: now + chrono::Duration::minutes(5),
            matchers: vec![],
            min_severity: Some(AlertLevel::Critical),
        };

        let alert_warn = make_alert("gateway", AlertLevel::Warning);
        let alert_crit = make_alert("gateway", AlertLevel::Critical);
        let alert_emerg = make_alert("gateway", AlertLevel::Emergency);

        // Warning < Critical，不应匹配
        assert!(!rule.matches_alert(&alert_warn));
        // Critical >= Critical，应匹配
        assert!(rule.matches_alert(&alert_crit));
        // Emergency >= Critical，应匹配
        assert!(rule.matches_alert(&alert_emerg));
    }

    #[test]
    fn test_manager_add_and_check() {
        let mut mgr = SilenceManager::new();
        let now = Utc::now();

        let rule = SilenceRule {
            id: "".into(),
            created_by: "test".into(),
            reason: "maintenance".into(),
            starts_at: now - chrono::Duration::minutes(1),
            ends_at: now + chrono::Duration::minutes(10),
            matchers: vec![SilenceLabelMatcher {
                key: "service".into(),
                value: "gateway".into(),
            }],
            min_severity: None,
        };

        mgr.add(rule);

        let alert = make_alert("gateway", AlertLevel::Warning);
        assert!(mgr.is_silenced(&alert).is_some());

        let alert_other = make_alert("auth", AlertLevel::Warning);
        assert!(mgr.is_silenced(&alert_other).is_none());
    }

    #[test]
    fn test_manager_remove() {
        let mut mgr = SilenceManager::new();
        let now = Utc::now();

        let rule = SilenceRule {
            id: "remove-me".into(),
            created_by: "test".into(),
            reason: "test".into(),
            starts_at: now - chrono::Duration::minutes(1),
            ends_at: now + chrono::Duration::minutes(10),
            matchers: vec![],
            min_severity: None,
        };

        let id = mgr.add(rule);
        assert_eq!(mgr.active_rules().len(), 1);

        mgr.remove(&id);
        assert_eq!(mgr.active_rules().len(), 0);
    }

    #[test]
    fn test_tick_expired() {
        let mut mgr = SilenceManager::new();
        let now = Utc::now();

        // 活跃的规则
        mgr.add(SilenceRule {
            id: "active".into(),
            created_by: "test".into(),
            reason: "active".into(),
            starts_at: now - chrono::Duration::minutes(1),
            ends_at: now + chrono::Duration::minutes(10),
            matchers: vec![],
            min_severity: None,
        });

        // 已过期的规则
        mgr.add(SilenceRule {
            id: "expired".into(),
            created_by: "test".into(),
            reason: "expired".into(),
            starts_at: now - chrono::Duration::minutes(30),
            ends_at: now - chrono::Duration::minutes(1),
            matchers: vec![],
            min_severity: None,
        });

        // 两个规则：一个活跃、一个已过期
        assert_eq!(mgr.all_rules().len(), 2);       // 所有规则（含未来）
        assert_eq!(mgr.active_rules().len(), 1);    // 活跃规则（现在时刻）
        let cleaned = mgr.tick_expired();

        assert_eq!(cleaned, 1);
        assert_eq!(mgr.all_rules().len(), 1);        // 只剩活跃的
        assert_eq!(mgr.active_rules().len(), 1);
        assert_eq!(mgr.expired_count(), 1);
    }

    #[test]
    fn test_maintenance_helper() {
        let mut mgr = SilenceManager::new();
        let now = Utc::now();

        let id = mgr.add_maintenance(
            now,
            now + chrono::Duration::hours(2),
            "数据库迁移",
            vec![("service", "gateway"), ("env", "prod")],
        );

        let id_from_mgr = {
            let rules = mgr.all_rules();
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].reason, "数据库迁移");
            assert_eq!(rules[0].matchers.len(), 2);
            rules[0].id.clone()
        };

        assert_eq!(id, id_from_mgr);
    }
}
