//! monitor-alert — 告警引擎
//!
//! 负责告警生命周期管理、规则评估、多通道通知分发。

pub mod manager;
pub mod notify;
pub mod router;
pub mod rule;
pub mod silence;
