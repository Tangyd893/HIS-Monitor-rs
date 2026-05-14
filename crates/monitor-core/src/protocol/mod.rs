//! Protobuf 协议定义
//!
//! 由 build.rs 通过 tonic-build 编译 proto 文件自动生成 Rust 代码。
//! 包含 monitor 和 alert 两个 package 的消息类型。
//!
//! 生成的代码仅依赖 `prost`，无需 `tonic` 运行时。

/// monitor package: MetricReport
pub mod monitor {
    include!(concat!(env!("OUT_DIR"), "/monitor.rs"));
}

/// alert package: AlertEvent
pub mod alert {
    include!(concat!(env!("OUT_DIR"), "/alert.rs"));
}
