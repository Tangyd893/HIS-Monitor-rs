# HIS-Monitor-rs

> [HIS-Go](https://github.com/Tangyd893/HIS-Go) 配套后台监控系统 — 基于 Rust 的全链路可观测性平台

HIS-Monitor-rs 是 HIS-Go 医院信息系统的**独立监控后端**，与业务系统解耦运行。覆盖 18 个微服务、17 个 PostgreSQL 库及全部基础设施（Redis / RabbitMQ / Nacos / MinIO / Nginx）。

## 状态

| 项目 | 指标 |
|------|------|
| 语言 | Rust 2024 edition |
| 异步运行时 | Tokio 1.x |
| 测试 | 76 passed · 0 failed |
| 前端 | 原生 HTML/CSS/JS 暗色大屏 |
| 许可证 | MIT |

## 项目结构

```
.
├── configs/                  # 配置文件
│   ├── monitor.yaml          # 默认配置
│   ├── monitor.dev.yaml      # 开发环境
│   └── monitor.prod.yaml     # 生产环境
├── crates/                   # Rust workspace 子 crate
│   ├── monitor-core/         # 核心模型、协议、配置、错误类型
│   ├── monitor-collector/    # 指标采集层（Prometheus scraper / OTLP receiver / exporters）
│   ├── monitor-processor/    # 数据处理层（聚合 / 异常检测 / 基线学习）
│   ├── monitor-alert/        # 告警引擎（规则评估 / 静默管理 / 通知分发 / 路由）
│   ├── monitor-api/          # REST API 服务（Axum）+ 运营大屏（web/）
│   └── monitor-probe/        # 合成监控探针引擎
├── web/                      # 前端监控大屏（单页应用）
├── deploy/                   # 部署配置
│   ├── docker/               # Dockerfile + docker-compose
│   ├── k8s/                  # Kubernetes 配置
│   └── dashboards/           # Grafana 仪表板 JSON
└── docs/                     # 架构设计文档 + 学习手册
```

## 快速开始

```bash
# 编译全部 crate
cargo build

# 运行测试（76 项）
cargo test

# 启动 API 服务
cargo run -p monitor-api -- --host 0.0.0.0 --port 3000

# 打开运营大屏
# 浏览器访问 http://localhost:3000
```

## 架构分层

```
┌─────────────────────────────────────────────────┐
│              HIS-Go 业务系统 (18 微服务)           │
└────────────────────┬────────────────────────────┘
                     │ HTTP Pull / OTLP gRPC
                     ▼
┌─────────────────────────────────────────────────┐
│              HIS-Monitor-rs (本系统)              │
│                                                  │
│  monitor-collector ──→ monitor-processor         │
│  (Prometheus scraper    (窗口聚合/异常检测)        │
│   管道处理/exporters)                             │
│       │                       │                  │
│       └───────┬───────────────┘                  │
│               ▼                                   │
│         monitor-api ───── monitor-alert          │
│         (Axum REST API)     (告警状态机/通知)      │
│         (运营大屏 SPA)                            │
│               │                                   │
│  monitor-probe (合成监控探活)                      │
└─────────────────────────────────────────────────┘
```

| 层级 | crate | 职责 | 技术亮点 |
|------|-------|------|---------|
| **采集层** | `monitor-collector` | Prometheus scraper、OTLP gRPC receiver、基础设施 exporter | 手写零拷贝解析器 · 管道模式 · trait 抽象导出器 |
| **处理层** | `monitor-processor` | 时间窗口聚合、Z-score / 固定阈值异常检测、基线学习 | 4 策略异常检测 · 滑动窗口基线 · epoch 对齐聚合 |
| **告警层** | `monitor-alert` | 多级规则引擎、静默窗口、钉钉/企微/邮件/Console 通知分发 | 有限状态机 · mpsc 事件驱动 · Arc<Mutex<>> 共享状态 |
| **API 层** | `monitor-api` | REST API（Axum）、运营大屏、Prometheus `/metrics` 端点 | Tower 中间件栈 · 限流/CORS/追踪 · 静态文件 SPA |
| **探活层** | `monitor-probe` | HTTP Chain 合成监控、断言验证、业务场景模拟 | 5 业务场景 · 变量传递 · 双 tick 调度 |

## API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/health` | 存活检查 → `{"status":"ok"}` |
| GET | `/metrics` | Prometheus 指标（已注册 4 个指标） |
| GET | `/api/v1/alerts` | 活跃告警列表（支持 status/level/service 过滤） |
| GET | `/api/v1/alerts/{id}` | 告警详情 |
| POST | `/api/v1/alerts/{id}/ack` | 确认告警（含 comment） |
| GET/POST | `/api/v1/silences` | 静默规则管理（标签匹配 + 时间窗口） |
| DELETE | `/api/v1/silences/{id}` | 删除静默规则 |
| GET/POST | `/api/v1/rules` | 告警规则管理（指标模式 / 比较操作 / 阈值） |
| GET | `/` | 运营监控大屏（暗色主题 SPA） |

## 运营大屏

单页应用，零依赖原生 HTML/CSS/JS，暗色监控主题：

- **顶部状态栏** — 实时时钟 · 系统健康徽章 · 活跃告警计数
- **左侧健康面板** — API 在线状态 · 活跃告警/静默/规则统计
- **左侧指标面板** — `/metrics` Prometheus 指标实时展示
- **告警表格** — 级别/状态筛选 · 一键确认(ack) · 级别/状态彩色标签
- **静默管理** — 模态框创建 · 标签匹配 · 时间窗口 · 删除
- **规则管理** — 模态框创建 · 比较操作选择 · 阈值/持续时间配置

## 设计原则

- **内存安全 & 零成本抽象** — Rust 所有权模型，无 GC 停顿
- **异步非阻塞** — Tokio 异步运行时，单节点支撑万级 QPS
- **标准协议兼容** — OpenTelemetry + Prometheus，对接 Grafana / Jaeger
- **独立部署** — 监控系统与 HIS-Go 完全解耦
- **分层解耦** — 采集→处理→存储→API→告警，各层独立水平扩展
- **分级告警** — Warning → Critical → Emergency，抑制与静默策略完整

## 深度阅读

- 📖 **[学习手册](docs/学习手册.md)** — 面试/学习导向的技术解读，含每层设计决策与代码示例
- 🏗️ **[架构设计文档](docs/HIS-Monitor-rs后台监控系统架构设计.md)** — 完整的系统架构设计

## License

MIT
