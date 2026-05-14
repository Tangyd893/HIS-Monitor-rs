//! 工具函数

/// 获取当前 Unix 时间戳（毫秒）
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
