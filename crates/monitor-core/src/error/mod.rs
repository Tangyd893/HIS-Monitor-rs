//! 统一错误类型

/// 监控系统全局错误枚举
#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    /// 配置错误
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    /// I/O 错误
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// HTTP 请求错误
    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    /// JSON 序列化/反序列化错误
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// 采集超时
    #[error("Collection timeout: {target}")]
    Timeout { target: String },

    /// 通用错误
    #[error("{0}")]
    Other(String),
}

/// 便捷 Result 类型别名
pub type MonitorResult<T> = Result<T, MonitorError>;
