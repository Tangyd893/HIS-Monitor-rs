# HIS-Monitor-rs 后台监控系统架构设计

> 本系统是 [HIS-Go](https://github.com/Tangyd893/HIS-Go)（`Tangyd893/HIS-Go`）的配套后台监控系统，基于 Rust 语言构建，负责对 HIS-Go 的 18 个微服务、17 个独立数据库及全部基础设施进行全链路可观测性覆盖。

---

## 一、项目定位与设计目标

### 1.1 项目定位

HIS-Monitor-rs 是 HIS-Go 医院信息系统的**独立监控后端**，与 HIS-Go 业务系统解耦运行。它不替代 Prometheus / Grafana / Jaeger / Alertmanager 等标准可观测性组件，而是在这些组件之上：

- 实现 **HIS 业务专有**的监控数据采集、处理与告警逻辑
- 提供 **高性能 Rust 原生**的探活代理、指标导出器、规则引擎
- 向上层（Grafana / Alertmanager / 运营管理后台）暴露统一的监控数据查询 API
- 承担 HIS-Go 暴露层 SDK 中**非侵入覆盖不到**的精细化监控需求（如合成监控、日志模式检测、异常检测）

### 1.2 监控目标

| 目标                       | 说明                                                         |
| -------------------------- | ------------------------------------------------------------ |
| **全链路可观测性**         | 覆盖 HIS-Go 全部 18 个微服务、17 个 PostgreSQL 库、Redis、RabbitMQ、Nacos、MinIO、Nginx |
| **故障快速发现与定位**     | 指标告警 + 分布式追踪 + 日志关联，分钟级问题响应             |
| **容量规划与趋势分析**     | 长期运行数据采集，支撑资源扩容决策                           |
| **符合医疗行业特点**       | 高可用、低延迟、数据安全、审计追溯                           |

### 1.3 设计原则

| 原则               | 说明                                                         |
| ------------------ | ------------------------------------------------------------ |
| **内存安全 & 零成本抽象** | 利用 Rust 所有权模型消除 GC 停顿与数据竞争，监控系统自身开销可控 |
| **异步非阻塞**     | 基于 Tokio 异步运行时，所有 I/O 操作异步化，单节点支撑万级 QPS 指标采集 |
| **标准协议兼容**   | 全链路采用 OpenTelemetry + Prometheus 标准，与 HIS-Go 及 Grafana/Jaeger 生态无缝对接 |
| **独立部署**       | 监控系统与 HIS-Go 业务系统完全解耦，监控链路故障不影响业务   |
| **分层解耦**       | 采集层 → 处理层 → 存储层 → API 层 → 告警层，各层可独立水平扩展 |
| **分级告警**       | Warning → Critical → Emergency 三级告警，抑制与静默策略完整  |
| **安全合规**       | 敏感医疗数据不在追踪和日志中暴露，端点访问受控               |

---

## 二、整体架构

### 2.1 架构全景图

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    被监控目标 (Targets — HIS-Go 业务系统)                   │
│                                                                          │
│   ┌──────────┐ ┌──────────┐ ┌──────────┐        ┌──────────┐            │
│   │Gateway   │ │ Auth     │ │ User     │  ...   │ EMR      │ (18服务)   │
│   │:8080     │ │:8081     │ │:8082     │        │:8097     │            │
│   └────┬─────┘ └────┬─────┘ └────┬─────┘        └────┬─────┘            │
│        │ /health    │ /metrics   │ /ready             │                  │
│        │ /ready     │ /debug/pprof│                   │                  │
│   ┌────┴────────────┴────────────┴───────────────────┴─────┐            │
│   │  PostgreSQL×17 │ Redis │ RabbitMQ │ Nacos │ MinIO │ Nginx │         │
│   └────────────────────────────────────────────────────────┘            │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │
                                   │ HTTP Pull / gRPC / AMQP / File Tail
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                   HIS-Monitor-rs 监控系统（本系统）                         │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    采集层 (Collector Layer)                       │    │
│  │                                                                   │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │    │
│  │  │metric-collector│  │trace-collector│  │ log-collector       │   │    │
│  │  │(Prometheus    │  │(OTLP gRPC    │  │(Fluentd/Tail/Audit)  │   │    │
│  │  │ scraper)      │  │ receiver)     │  │                      │   │    │
│  │  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘   │    │
│  │         │                 │                      │               │    │
│  │  ┌──────┴─────────────────┴──────────────────────┴───────────┐  │    │
│  │  │                   基础设施 Exporters                        │  │    │
│  │  │  pg-exporter │ redis-exporter │ mq-exporter │ nacos-exp.  │  │    │
│  │  └───────────────────────────────────────────────────────────┘  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                   │                                      │
│                                   ▼                                      │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    处理层 (Processor Layer)                       │    │
│  │                                                                   │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │    │
│  │  │metric-processor│ │rule-engine   │  │ anomaly-detector     │   │    │
│  │  │(聚合/降采样/   │  │(告警规则     │  │(基线学习/            │   │    │
│  │  │ 预计算)        │  │ 评估引擎)    │  │ 异常检测)            │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                   │                                      │
│                                   ▼                                      │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                     存储层 (Storage Layer)                        │    │
│  │                                                                   │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │    │
│  │  │VictoriaMetrics│  │  Jaeger/Qryn │  │ Elasticsearch / Loki  │   │    │
│  │  │(时序指标)     │  │  (链路追踪)   │  │  (日志)              │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │    │
│  │  ┌──────────────┐  ┌──────────────────────────────────────────┐ │    │
│  │  │ PostgreSQL    │  │  MinIO (冷数据归档、审计快照)             │ │    │
│  │  │ (告警记录/    │  │                                          │ │    │
│  │  │  配置/审计)   │  │                                          │ │    │
│  │  └──────────────┘  └──────────────────────────────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                   │                                      │
│                                   ▼                                      │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                     API 层 (API Layer)                            │    │
│  │                                                                   │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │    │
│  │  │ REST API      │  │ gRPC API     │  │ WebSocket (push)     │   │    │
│  │  │ (Axum)        │  │ (Tonic)      │  │ (实时告警推送)       │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                   │                                      │
│                                   ▼                                      │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    告警层 (Alert Layer)                           │    │
│  │                                                                   │    │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │    │
│  │  │alert-manager  │  │notify-dispatcher│ │silence-scheduler   │   │    │
│  │  │(告警路由/     │  │(多渠道通知:   │  │(静默窗口/           │   │    │
│  │  │ 分组/抑制)    │  │ 电话/钉钉/    │  │ 维护模式)           │   │    │
│  │  │               │  │ 企微/邮件/   │  │                      │   │    │
│  │  │               │  │ 短信)        │  │                      │   │    │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                    展示层 (Presentation — 外部系统)                        │
│                                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────────┐   │
│  │  Grafana     │  │ Alertmanager │  │  HIS-Go 运营管理后台 (Vue3)   │   │
│  │  (仪表盘)    │  │  (上游告警)  │  │  (IFrame嵌入 + API对接)       │   │
│  └──────────────┘  └──────────────┘  └──────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                    链路检测层 (Synthetic Monitoring)                       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  monitor-probe (Rust)                                             │   │
│  │  定时执行: 模拟挂号 → 模拟就诊 → 模拟开处方 → 模拟收费 → 模拟发药 │   │
│  │  记录每环节延迟 & 成功率 → 暴露为 Prometheus 指标                  │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────┘
```

### 2.2 架构说明

- **采集层**：HIS-Monitor-rs 同时具备主动拉取（Prometheus Scrape）和被动接收（OTLP gRPC / Fluentd Forward）两种采集能力，以适配 HIS-Go 不同服务的暴露形式
- **处理层**：在指标写入存储前完成聚合降采样、规则评估和异常检测，减少存储写入放大并提升告警时效
- **存储层**：按数据类型分流 —— 时序指标写入 VictoriaMetrics、追踪数据写入 Jaeger/Qryn、日志写入 Elasticsearch/Loki、系统自身配置与告警记录写入 PostgreSQL
- **API 层**：统一通过 Axum 提供 RESTful API，Tonic 提供 gRPC API，WebSocket 推送实时告警
- **告警层**：自建告警路由引擎，支持与 Prometheus Alertmanager 协同工作，亦可独立运行
- **探活层**：以独立 Rust 二进制运行的合成监控代理，模拟 HIS 核心业务链路

---

## 三、技术选型

### 3.1 Rust 生态选型

| 领域           | 组件                         | 版本    | 用途                                   |
| -------------- | ---------------------------- | ------- | -------------------------------------- |
| **异步运行时** | Tokio                        | 1.x     | 多线程异步运行时，支撑万级并发连接     |
| **HTTP 框架**  | Axum                         | 0.8+    | REST API 服务，基于 Tower 中间件体系   |
| **gRPC 框架**  | Tonic                        | 0.12+   | gRPC Server/Client，原生 Prost 集成    |
| **序列化**     | Serde + Prost                | —       | JSON/YAML 序列化 + Protobuf 编解码     |
| **数据库**     | SQLx + deadpool-postgres     | 0.8+    | 异步 PostgreSQL 连接池，编译期 SQL 校验 |
| **缓存**       | redis-rs (fred)              | 10.x    | Redis 异步客户端，支持集群/哨兵        |
| **消息队列**   | lapin                        | 2.x     | RabbitMQ AMQP 0-9-1 异步客户端         |
| **日志**       | tracing + tracing-subscriber | 0.1/0.3 | 结构化日志 + Span 追踪，兼容 OpenTelemetry |
| **指标**       | opentelemetry-rust + metrics | 0.28+   | OpenTelemetry SDK + Prometheus 指标暴露 |
| **配置**       | config-rs + nacos-rs         | 0.15+   | 多源配置加载 + Nacos 配置中心热更新    |
| **序列编排**   | Temporal Rust SDK / 自研     | —       | 探活任务的 DAG 编排与重试              |
| **异常检测**   | augurs (Rust)                | —       | 时序异常检测库（可选集成）             |
| **WebSocket**  | tokio-tungstenite / Axum WS  | —       | 实时告警推送                           |
| **HTTP 客户端**| reqwest                      | 0.12+   | 异步 HTTP 客户端（探活、API 调用）     |
| **模板引擎**   | askama / tera                | —       | 告警消息模板渲染                       |
| **加密**       | ring / rustls                | —       | TLS 通信、JWT 验证、密码哈希           |

### 3.2 外部集成组件

| 组件               | 角色                                 | 与 HIS-Monitor-rs 的交互           |
| ------------------ | ------------------------------------ | ---------------------------------- |
| Prometheus         | 指标抓取引擎                         | HIS-Monitor-rs 暴露 /metrics 端点  |
| VictoriaMetrics    | 长周期时序存储                       | Prometheus remote_write / HTTP API |
| Jaeger / Qryn      | 分布式追踪存储与查询                 | OTLP gRPC 接收 Trace               |
| Grafana            | 统一可视化面板                       | 读取 Prometheus/Jaeger/ES 数据源   |
| Alertmanager       | 上游告警路由（可选）                 | 接收 HIS-Monitor-rs 推送的告警     |
| Elasticsearch/Loki | 日志存储与检索                       | Fluentd/Promtail 采集后写入        |
| Nacos              | 配置中心 & 服务发现                  | 动态配置热更新                     |
| MinIO              | 冷数据归档、审计快照、仪表盘 JSON    | S3 兼容 API                        |

---

## 四、Cargo Workspace 工程结构

HIS-Monitor-rs 采用 Cargo Workspace 管理多个子 crate，按功能域垂直切分：

```
HIS-Monitor-rs/
├── Cargo.toml                        # Workspace 根配置
├── Cargo.lock
├── configs/                          # 配置文件
│   ├── monitor.yaml                  # 默认配置
│   ├── monitor.dev.yaml              # 开发环境
│   └── monitor.prod.yaml             # 生产环境
├── docker/                           # Docker 部署
│   ├── Dockerfile                    # 多阶段构建
│   ├── docker-compose.monitoring.yml # 监控栈编排
│   └── .env.example                  # 环境变量模板
├── dashboards/                       # Grafana 仪表盘 JSON
│   └── his-overview.json
├── docs/                             # 项目文档
├── k8s/                              # Kubernetes 部署清单
│   └── monitoring/
│       ├── namespace.yaml
│       ├── configmap.yaml
│       ├── prometheus.yaml
│       ├── monitor-core.yaml         # HIS-Monitor-rs 核心服务
│       └── monitor-probe.yaml        # 探活代理
│
├── crates/
│   ├── monitor-core/                 # 核心基础库
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config/               # 配置加载 (Nacos + 本地文件)
│   │       ├── error/                # 统一错误类型
│   │       ├── model/                # 共享数据模型
│   │       │   ├── metric.rs         # 指标模型
│   │       │   ├── trace.rs          # 追踪模型
│   │       │   ├── alert.rs          # 告警模型
│   │       │   └── health.rs         # 健康检查模型
│   │       ├── protocol/             # Protobuf 定义
│   │       │   └── proto/
│   │       │       ├── monitor.proto # 监控数据协议
│   │       │       └── alert.proto   # 告警协议
│   │       └── util/                 # 工具函数
│   │
│   ├── monitor-collector/            # 数据采集器
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── scraper/              # Prometheus 指标抓取器
│   │       ├── receiver/             # OTLP gRPC 追踪接收器
│   │       ├── pipeline/             # 采集流水线 (过滤/转换/采样)
│   │       └── exporter/             # 基础设施 Exporter
│   │           ├── postgres.rs       # PostgreSQL 监控
│   │           ├── redis.rs          # Redis 监控
│   │           ├── rabbitmq.rs       # RabbitMQ 监控
│   │           ├── nacos.rs          # Nacos 监控
│   │           └── minio.rs          # MinIO 监控
│   │
│   ├── monitor-processor/            # 数据处理引擎
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── aggregator/           # 指标聚合降采样
│   │       ├── anomaly/              # 异常检测
│   │       │   ├── mod.rs
│   │       │   ├── baseline.rs       # 基线学习
│   │       │   └── detector.rs       # 异常检测算法
│   │       └── stream/               # 流式处理
│   │
│   ├── monitor-alert/                # 告警引擎
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── rule/                 # 告警规则
│   │       │   ├── engine.rs         # 规则评估引擎
│   │       │   ├── parser.rs         # 规则表达式解析 (PromQL/LogQL 子集)
│   │       │   └── template.rs       # 告警消息模板
│   │       ├── manager.rs            # 告警生命周期管理 (触发/恢复/静默)
│   │       ├── router.rs             # 告警路由与分组
│   │       ├── silence.rs            # 静默窗口管理
│   │       └── notify/               # 通知通道适配器
│   │           ├── mod.rs
│   │           ├── dingtalk.rs       # 钉钉
│   │           ├── wecom.rs          # 企业微信
│   │           ├── email.rs          # 邮件
│   │           ├── sms.rs            # 短信
│   │           └── phone.rs          # 电话 (通过第三方 API)
│   │
│   ├── monitor-probe/                # 业务链路探活代理
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs               # 独立二进制入口
│   │       ├── engine/               # 探活编排引擎
│   │       ├── scenario/             # 业务场景定义
│   │       │   ├── registration.rs   # 挂号场景
│   │       │   ├── clinic.rs         # 就诊场景
│   │       │   ├── prescription.rs   # 处方场景
│   │       │   ├── billing.rs        # 收费场景
│   │       │   └── pharmacy.rs       # 发药场景
│   │       └── metrics.rs            # 探活指标暴露
│   │
│   └── monitor-api/                  # 对外 API 服务
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs               # 独立二进制入口
│           ├── server/               # 服务器启动与中间件
│           ├── route/                # 路由定义
│           │   ├── health.rs         # 健康检查 API
│           │   ├── metric.rs         # 指标查询 API
│           │   ├── alert.rs          # 告警管理 API
│           │   ├── probe.rs          # 探活控制 API
│           │   └── config.rs         # 配置管理 API
│           ├── handler/              # 请求处理器
│           ├── middleware/           # 中间件 (认证/限流/日志)
│           └── ws/                   # WebSocket 实时推送
│
├── tests/                            # 集成测试
│   └── integration/
└── scripts/                          # 辅助脚本
    ├── check.sh                      # Rust 代码质量检查
    └── docker-run.sh                 # 本地 Docker 启动
```

### 4.1 Crate 职责与依赖关系

```
                        ┌─────────────┐
                        │ monitor-api │  (独立二进制)
                        └──────┬──────┘
                               │ 依赖
               ┌───────────────┼───────────────┐
               ▼               ▼               ▼
        ┌────────────┐ ┌────────────┐ ┌────────────┐
        │monitor-    │ │monitor-    │ │monitor-    │
        │collector   │ │processor   │ │alert       │
        └──────┬─────┘ └──────┬─────┘ └──────┬─────┘
               │              │              │
               └──────────────┼──────────────┘
                              │ 依赖
                              ▼
                      ┌──────────────┐
                      │ monitor-core │  (基础库)
                      └──────────────┘

        ┌─────────────┐
        │monitor-probe│  (独立二进制，仅依赖 monitor-core)
        └─────────────┘
```

- **monitor-core**：无状态基础库，定义共享数据模型、Proto 协议、配置加载、错误类型、工具函数
- **monitor-collector**：网络密集型，负责对接外部指标端点，依赖 monitor-core
- **monitor-processor**：CPU 密集型，负责数据聚合与计算，依赖 monitor-core
- **monitor-alert**：规则引擎 + 外部通知，依赖 monitor-core
- **monitor-probe**：独立二进制，依赖 monitor-core，不与其他模块耦合
- **monitor-api**：独立二进制，集成所有模块，作为对外统一入口

---

## 五、核心模块详细设计

### 5.1 数据采集模块 (monitor-collector)

#### 5.1.1 架构

```
                            ┌──────────────────────────┐
                            │    Service Registry       │
                            │  (Nacos / Static Config)  │
                            └────────────┬─────────────┘
                                         │ 服务发现
                                         ▼
┌────────────────────────────────────────────────────────────────────┐
│                       Scraper Manager (Tokio)                       │
│                                                                     │
│   ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────┐  │
│   │  HTTP Scraper    │   │  gRPC Receiver   │   │  Log Tailer     │  │
│   │  (reqwest)       │   │  (Tonic)         │   │  (file/inotify) │  │
│   │                  │   │                  │   │                 │  │
│   │  定时 Pull:      │   │  被动接收:       │   │  监控 HIS-Go    │  │
│   │  GET /metrics    │   │  OTLP gRPC Stream│   │  容器日志文件   │  │
│   │  GET /health     │   │  Span/Log 数据   │   │                 │  │
│   └────────┬────────┘   └────────┬────────┘   └────────┬────────┘  │
│            │                     │                     │            │
│            └─────────────────────┼─────────────────────┘            │
│                                  ▼                                  │
│                      ┌───────────────────────┐                      │
│                      │   Collector Pipeline   │                      │
│                      │  • 过滤 (filter)       │                      │
│                      │  • 变换 (transform)    │                      │
│                      │  • 采样 (sample)       │                      │
│                      │  • 富化 (enrich,       │                      │
│                      │    追加 HIS 业务标签)  │                      │
│                      └───────────┬───────────┘                      │
│                                  │                                  │
│                   ┌──────────────┼──────────────┐                   │
│                   ▼              ▼              ▼                   │
│            ┌───────────┐ ┌───────────┐ ┌──────────────┐            │
│            │Victoria-   │ │  Jaeger   │ │Elasticsearch │            │
│            │Metrics     │ │  (OTLP)   │ │/Loki         │            │
│            │(remote_w)  │ │           │ │              │            │
│            └───────────┘ └───────────┘ └──────────────┘            │
└────────────────────────────────────────────────────────────────────┘
```

#### 5.1.2 采集目标清单

| 采集对象                  | 采集方式                | 端口/端点                        | 采集间隔 |
| ------------------------- | ----------------------- | -------------------------------- | -------- |
| HIS-Go 18 微服务 /metrics | HTTP Pull (Prometheus)  | `:8080~8097/metrics`             | 15s      |
| HIS-Go 健康检查           | HTTP Pull               | `:8080~8097/health`, `/ready`    | 10s      |
| HIS-Go gRPC 指标          | gRPC Interceptor 暴露   | `:9081~9097` (内置 metrics)      | 15s      |
| PostgreSQL (×17)          | postgres_exporter / SQL | `:9187/metrics`                  | 30s      |
| Redis                     | redis_exporter          | `:9121/metrics`                  | 15s      |
| RabbitMQ                  | rabbitmq_exporter       | `:9419/metrics`                  | 15s      |
| Nacos                     | Nacos OpenAPI           | `:8848/nacos/v1/ns/operator/metrics` | 30s |
| MinIO                     | minio_exporter          | `:9000/minio/v2/metrics/cluster` | 30s      |
| Nginx                     | nginx stub_status       | `:80/nginx_status`               | 15s      |
| OTLP Trace                | gRPC Stream (被动接收)  | `:4317`                          | 实时     |
| Docker 容器日志           | File Tail / Fluentd     | `/var/lib/docker/containers/**/*.log` | 实时 |

#### 5.1.3 自定义 HIS 基础设施 Exporter

针对 HIS-Go 无法直接使用社区 Exporter 的场景，在 Rust 中自研：

| Exporter           | 说明                                          | 核心指标                                  |
| ------------------ | --------------------------------------------- | ----------------------------------------- |
| `his-pg-exporter`  | 按服务维度聚合 17 个 DB 的连接/慢查询/锁等待  | 连接数、慢查询 TOP N、死锁次数、主从延迟  |
| `his-redis-exporter` | 号源缓存命中率、排队队列深度                  | 缓存命中率、队列长度、锁等待时间          |
| `his-mq-exporter`  | 按交换机/队列维度的消息积压与死信统计         | 消息速率、积压数量、死信数量、确认延迟    |
| `his-nacos-exporter` | 服务实例上下线事件统计                      | 注册实例数、心跳延迟、配置推送次数        |
| `his-nginx-exporter` | HIS 特有路由 (挂号/处方等) 的请求量分布      | 按 path 统计请求量、4xx/5xx 比例、延迟    |

Exporter 统一注册到 Prometheus 的 `scrape_configs` 中，通过 Nacos 服务发现或 `static_configs` 指定。

---

### 5.2 数据处理模块 (monitor-processor)

#### 5.2.1 数据流水线

```
原始指标/日志 ──→ [预过滤] ──→ [标签富化] ──→ [聚合窗口] ──→ [降采样] ──→ 写入存储
                                     │
                                     ├──→ [规则评估] ──→ 触发告警
                                     └──→ [异常检测] ──→ 异常事件
```

**处理流程**：

1. **预过滤**：丢弃无关注的指标（如 `/debug/pprof/` 自身指标），减少写入放大
2. **标签富化**：为来自 HIS-Go 的指标自动追加 `cluster`、`env`、`region` 等标签（通过 Nacos/ConfigMap 注入）
3. **聚合窗口**：对 Counter 类指标做滚动窗口聚合（1min / 5min / 15min），预计算 `rate()`、`increase()` 等
4. **降采样**：原始 15s 粒度的指标数据在 7 天后降采样为 5min 粒度，30 天后降采样为 1h 粒度
5. **规则评估**：实时评估告警规则，命中后推送告警事件到 monitor-alert
6. **异常检测**：基于历史基线（同环比）检测指标异常波动

#### 5.2.2 异常检测策略

| 方法           | 适用场景                  | 说明                               |
| -------------- | ------------------------- | ---------------------------------- |
| 固定阈值       | 明确告警条件的场景        | 如"服务错误率 > 5%"                |
| 同环比波动     | 业务指标（挂号量、处方量）| 当日值相比昨日同时段下降 > 50%     |
| 标准差偏离     | 延迟类指标                | P99 延迟超过历史基线 3 倍标准差    |
| 突变检测       | 流量突增/突降             | 5 分钟内变化率超过 200%            |

---

### 5.3 告警引擎模块 (monitor-alert)

#### 5.3.1 告警生命周期

```
   ┌─────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
   │ PENDING │────▶│ FIRING   │────▶│ ACKED    │────▶│ RESOLVED │
   │ 待触发   │     │ 已触发    │     │ 已确认    │     │ 已恢复    │
   └─────────┘     └────┬─────┘     └──────────┘     └──────────┘
                        │
                        ├── 抑制规则命中 ──→ 降级或忽略
                        ├── 静默窗口命中 ──→ 延迟通知
                        └── 通知通道分发 ──→ 钉钉/企微/邮件/短信/电话
```

#### 5.3.2 告警规则定义

告警规则以 YAML 文件定义，支持热加载（通过 Nacos 或文件监听）：

```yaml
# 示例：挂号服务错误率告警
- name: his-registration-error-rate
  description: "挂号服务错误率超限"
  severity: critical
  expr: |
    rate(his_registration_total{status="error"}[5m])
    /
    rate(his_registration_total[5m])
    > 0.05
  for: 2m
  labels:
    service: registration
    category: business
  annotations:
    summary: "挂号服务错误率超过 5%"
    description: "当前错误率 {{ $value | humanizePercentage }}，部门：{{ $labels.department }}"
  routes:
    - channel: wecom
      receivers: ["his-oncall-group"]
    - channel: email
      receivers: ["devops@hospital.com"]
  inhibitors:
    - service_down{service="registration"}   # 若服务宕机则抑制本告警
```

#### 5.3.3 告警通知通道矩阵

| 级别           | 通道                            | 示例场景                                     |
| -------------- | ------------------------------- | -------------------------------------------- |
| P0 - Emergency | 电话 + 企微 @all + 钉钉群       | 挂号/处方服务整体不可用、DB 连接池耗尽       |
| P1 - Critical  | 企微 + 钉钉群 + 邮件             | 服务错误率 > 5%、gRPC P99 > 1s、支付超时     |
| P2 - Warning   | 邮件                             | Redis 内存 > 80%、消息积压 > 1000            |
| P3 - Info      | 企微机器人 (静默群)             | 服务上下线、配置变更通知                     |

#### 5.3.4 告警抑制与聚合

- **抑制规则**：当某服务整体 Down 时，抑制其所有子维度告警（如 DB 连接、Redis 缓存、gRPC 调用）
- **静默窗口**：计划维护期间自动静默，通过 API 或配置文件预定义
- **告警聚合**：同一服务 5 分钟内的同类告警合并为一条通知，避免告警风暴（令牌桶限流）

---

### 5.4 业务链路探活模块 (monitor-probe)

#### 5.4.1 设计目标

monitor-probe 是 HIMonitor-rs 中**唯一独立运行的二进制**，模拟真实用户行为，以黑盒视角验证 HIS-Go 核心业务链路的端到端健康状态。

#### 5.4.2 探活场景编排

```
                    ┌─────────────────────────────────┐
                    │        Probe Engine (自研)       │
                    │  基于 DAG 的任务编排 & 上下文传递  │
                    └─────────────────────────────────┘
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          ▼                         ▼                         ▼
   ┌────────────────┐    ┌────────────────┐    ┌────────────────┐
   │ 场景: 挂号链路  │    │ 场景: 处方链路  │    │ 场景: 收费链路  │
   │                 │    │                 │    │                 │
   │ 1. 获取排班信息  │    │ 1. 患者挂号      │    │ 1. 查询待收费单  │
   │ 2. 提交挂号请求  │    │ 2. 医生接诊      │    │ 2. 提交支付请求  │
   │ 3. 验证挂号结果  │    │ 3. 开具处方      │    │ 3. 验证支付结果  │
   │ 4. 记录延迟/状态 │    │ 4. 处方审核      │    │ 4. 记录延迟/状态 │
   └────────┬───────┘    │ 5. 记录延迟/状态 │    └────────┬───────┘
            │            └────────┬───────┘             │
            │                     │                      │
            └─────────────────────┼──────────────────────┘
                                  ▼
                    ┌────────────────────────────┐
                    │  结果写入 Prometheus Gauge   │
                    │                             │
                    │  his_probe_success          │
                    │    {chain, step}            │
                    │  his_probe_duration_seconds │
                    │    {chain, step}            │
                    └────────────────────────────┘
```

#### 5.4.3 技术实现要点

- 通过 `reqwest` 异步 HTTP 客户端调用 HIS-Go Gateway (`:8080`)，携带 JWT Token
- 探活间隔可配置（默认 30s），关键链路（挂号/处方）支持更短间隔（10s）
- 步骤间上下文传递（如挂号返回的 `registration_id` 传递给后续步骤）
- 每个场景的每个步骤独立计时，成功/失败状态以 Prometheus Gauge 暴露
- 探活代理自身健康状态通过 `/health` 和 `/metrics` 端点暴露，纳入 Prometheus 抓取

#### 5.4.4 探活指标示例

```
# 挂号链路探活成功
his_probe_success{chain="registration", step="submit"} 1
# 处方链路探活延迟
his_probe_duration_seconds{chain="prescription", step="audit"} 0.452
# 收费链路支付超时（失败）
his_probe_success{chain="billing", step="pay"} 0
```

---

### 5.5 API 服务模块 (monitor-api)

#### 5.5.1 API 设计

monitor-api 是 HIS-Monitor-rs 对外的唯一入口，提供：

| 端点分组                        | 方法      | 路径                                | 说明                       |
| ------------------------------- | --------- | ----------------------------------- | -------------------------- |
| **健康检查**                    | GET       | `/health`                           | 存活检查                   |
|                                 | GET       | `/ready`                            | 就绪检查 (含 DB/Redis/)    |
|                                 | GET       | `/metrics`                          | Prometheus 指标暴露        |
| **指标查询**                    | GET       | `/api/v1/metrics/query`             | 即时查询 (PromQL 兼容)     |
|                                 | GET       | `/api/v1/metrics/range`             | 范围查询                   |
|                                 | GET       | `/api/v1/metrics/labels`            | 标签列表                   |
| **告警管理**                    | GET       | `/api/v1/alerts`                    | 活跃告警列表               |
|                                 | POST      | `/api/v1/alerts/silence`            | 创建静默窗口               |
|                                 | DELETE    | `/api/v1/alerts/silence/{id}`       | 删除静默窗口               |
|                                 | PUT       | `/api/v1/alerts/{id}/ack`           | 确认告警                   |
| **探活管理**                    | GET       | `/api/v1/probe/status`              | 探活代理状态               |
|                                 | GET       | `/api/v1/probe/results`             | 探活历史结果               |
|                                 | POST      | `/api/v1/probe/trigger`             | 手动触发探活               |
| **配置管理**                    | GET       | `/api/v1/config`                    | 当前配置                   |
|                                 | PUT       | `/api/v1/config/reload`             | 热重载配置 (Nacos 回调)    |
| **WebSocket**                   | WS        | `/ws/alerts`                        | 实时告警推送               |

#### 5.5.2 中间件链

```
Request
  → Tracing (OpenTelemetry Span)
  → CORS (tower-http)
  → RateLimit (令牌桶)
  → Auth (JWT / API Key 验证)
  → Logger (tracing)
  → Handler
  → Response
```

---

## 六、监控指标体系设计

### 6.1 四类黄金信号

| 信号类型               | 核心指标                              | 数据来源                         |
| ---------------------- | ------------------------------------- | -------------------------------- |
| **延迟 (Latency)**     | HTTP/gRPC 请求 P50/P95/P99 延迟       | Gin 中间件 + gRPC Interceptor    |
| **流量 (Traffic)**     | 每秒请求量 (QPS)、并发连接数          | Gin 中间件 + Nginx               |
| **错误 (Errors)**      | 4xx/5xx 错误率、gRPC 错误码分布       | Gin 中间件 + gRPC Interceptor    |
| **饱和度 (Saturation)** | Goroutine 数量、内存使用率、DB 连接池 | runtime/metrics + Exporter       |

### 6.2 HIS 业务特有指标

| 指标名称                        | 说明                         | 标签                           |
| ------------------------------- | ---------------------------- | ------------------------------ |
| `his_registration_total`        | 挂号总量                     | department, status, source     |
| `his_outpatient_visit_total`    | 门诊就诊量                   | department, doctor             |
| `his_prescription_total`        | 处方开具量                   | type, department               |
| `his_billing_total`             | 收费笔数                     | pay_method, status             |
| `his_pharmacy_dispense_total`   | 发药量                       | drug_type, window              |
| `his_inpatient_admission_total` | 入院人数                     | dept, urgent                   |
| `his_cdss_alert_total`          | CDSS 告警次数                | alert_type, severity           |
| `his_emr_soap_create_total`     | SOAP 病历创建量              | department                     |
| `his_probe_success`             | 探活成功状态 (黑盒)          | chain, step                    |
| `his_probe_duration_seconds`    | 探活耗时 (黑盒)              | chain, step                    |

### 6.3 告警规则覆盖矩阵

| 监控对象     | 规则数      | 示例                                                       |
| ------------ | ----------- | ---------------------------------------------------------- |
| 微服务健康   | 18×3 = 54   | 服务存活、错误率 (5min > 5%)、gRPC P99 > 1s               |
| PostgreSQL   | 17×4 = 68   | 连接池 > 85%、慢查询 > 200ms、磁盘 > 80%、主从延迟 > 5s    |
| Redis        | 6           | 内存 > 80%、命中率 < 90%、连接数 > 80%、主从延迟           |
| RabbitMQ     | 5           | 积压 > 1000、死信 > 10 / 5min、连接数、内存 > 80%          |
| Nginx        | 4           | 4xx > 5%、5xx > 1%、P95 > 2s、连接数                       |
| Nacos        | 3           | 实例数变化、配置推送异常、心跳超时                         |
| MinIO        | 3           | 容量 > 80%、请求错误 > 5%、延迟 > 1s                       |
| 业务链路     | 5           | 挂号成功率、处方/收费端到端延迟、住院链路可用性            |
| K8s 基础设施 | 8           | Pod 重启、Node 资源、PV 容量、OOM Kill                     |

---

## 七、与 HIS-Go 的集成设计

### 7.1 HIS-Go 暴露层要求

HIS-Monitor-rs 的正常运作要求 HIS-Go 各微服务暴露以下端点（当前 HIS-Go 已具备 `/health` 和 `/ready`）：

```
每个 HIS-Go 微服务需暴露：
  GET  /health          # 存活检查（已具备）
  GET  /ready           # 就绪检查 (含 DB/Redis 连通性，已具备)
  GET  /metrics         # Prometheus 指标（需 HIS-Go 注入 promhttp）
  GET  /debug/pprof/    # 运行时诊断（生产环境需 IP 白名单）

gRPC 拦截器需注入：
  - gRPC Server/Client Metrics Interceptor
  - OpenTelemetry Trace Context 传播

日志需输出：
  - Zap 结构化 JSON 日志 (含 trace_id, span_id, service_name)
```

### 7.2 Prometheus 服务发现

HIS-Monitor-rs 的 scraper 通过以下方式发现 HIS-Go 各服务的 `/metrics` 端点：

- **Docker Compose 环境**：`static_configs` 方式指定容器名+端口
- **K8s 环境**：通过 `kubernetes_sd_configs` 自动发现带有 `prometheus.io/scrape: "true"` 注解的 Pod
- **Nacos 注册中心**：通过 Nacos OpenAPI 拉取所有注册服务实例，动态生成 scrape targets

### 7.3 与上游系统的时序交互

```
                    HIS-Go 微服务
                         │
              ┌──────────┼──────────┐
              ▼          ▼          ▼
         /metrics   OTLP Trace   Zap Logs
              │          │          │
              │          │          │
    ┌─────────┴──────────┴──────────┴─────────┐
    │           HIS-Monitor-rs                 │
    │                                          │
    │  collector ──→ processor ──→ storage     │
    │                      │                   │
    │                      └──→ alert ──→ 通知  │
    │     ▲                ▲                   │
    │     │ monitor-api    │ monitor-probe     │
    └─────┼────────────────┼───────────────────┘
          │                │
          ▼                ▼
    ┌──────────┐   ┌──────────────┐
    │ Grafana  │   │ Alertmanager │
    └──────────┘   └──────────────┘
          ▲
          │ IFrame 嵌入
    ┌─────┴──────────────┐
    │ HIS-Go 运营管理后台 │
    └────────────────────┘
```

---

## 八、部署架构

### 8.1 Docker Compose 部署

在 HIS-Go 现有 `docker-compose.yml` 基础上，新增监控服务栈。监控栈独立于业务栈，放在单独的 `docker-compose.monitoring.yml`：

```yaml
version: '3.8'

services:
  # ============ HIS-Monitor-rs 核心服务 ============
  monitor-api:
    build:
      context: ./crates/monitor-api
      dockerfile: ../../docker/Dockerfile
    image: his-monitor/api:latest
    ports:
      - "9100:9100"     # REST API
      - "9101:9101"     # /metrics (供 Prometheus 抓取)
    environment:
      - MONITOR_ENV=production
      - NACOS_ADDR=nacos:8848
      - REDIS_URL=redis://his-redis:6379
      - DATABASE_URL=postgresql://his_admin:${DB_PASSWORD}@his-postgres:5432/his_system
    volumes:
      - ./configs:/app/configs
    depends_on:
      - postgresql
      - redis
      - nacos

  monitor-probe:
    build:
      context: ./crates/monitor-probe
      dockerfile: ../../docker/Dockerfile
    image: his-monitor/probe:latest
    environment:
      - GATEWAY_URL=http://gateway:8080
      - PROBE_INTERVAL=30s
      - PROBE_CRITICAL_INTERVAL=10s
    depends_on:
      - gateway

  # ============ 标准监控组件 ============
  prometheus:
    image: prom/prometheus:v3.0
    ports:
      - "9090:9090"
    volumes:
      - ./docker/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml
      - ./docker/prometheus/alerts.yml:/etc/prometheus/alerts.yml
      - prometheus_data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=30d'
      - '--web.enable-remote-write-receiver'

  victoriametrics:
    image: victoriametrics/victoria-metrics:latest
    ports:
      - "8428:8428"
    volumes:
      - victoria_metrics_data:/victoria-metrics-data
    command:
      - '-retentionPeriod=12'
      - '-storageDataPath=/victoria-metrics-data'

  grafana:
    image: grafana/grafana:11.0
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=${GRAFANA_PASSWORD}
    volumes:
      - ./dashboards:/etc/grafana/provisioning/dashboards
      - ./docker/grafana/datasources:/etc/grafana/provisioning/datasources
      - grafana_data:/var/lib/grafana

  jaeger:
    image: jaegertracing/all-in-one:latest
    ports:
      - "16686:16686"
      - "14250:14250"
      - "4317:4317"
      - "4318:4318"
    environment:
      - SPAN_STORAGE_TYPE=badger
      - BADGER_EPHEMERAL=false
      - BADGER_DIRECTORY_VALUE=/badger/data
      - BADGER_DIRECTORY_KEY=/badger/key

  # ============ 基础设施 Exporter ============
  postgres-exporter:
    image: prometheuscommunity/postgres-exporter:latest
    ports:
      - "9187:9187"
    environment:
      - DATA_SOURCE_NAME=postgresql://his_admin:${DB_PASSWORD}@his-postgres:5432/?sslmode=disable

  redis-exporter:
    image: oliver006/redis_exporter:latest
    ports:
      - "9121:9121"
    command: ["-redis.addr", "redis://his-redis:6379"]

  rabbitmq-exporter:
    image: kbudde/rabbitmq-exporter:latest
    ports:
      - "9419:9419"
    environment:
      - RABBIT_URL=http://his-rabbitmq:15672

volumes:
  prometheus_data:
  grafana_data:
  victoria_metrics_data:
```

### 8.2 K8s 部署

```
k8s/monitoring/
├── namespace.yaml                     # monitoring 命名空间
├── monitor-core/
│   ├── configmap.yaml                 # HIS-Monitor-rs 配置
│   ├── deployment.yaml                # monitor-api Deployment
│   ├── service.yaml                   # Service (ClusterIP)
│   └── hpa.yaml                       # 水平自动扩缩容
├── monitor-probe/
│   └── deployment.yaml                # monitor-probe Deployment (单副本)
├── prometheus/
│   ├── configmap.yaml
│   ├── deployment.yaml
│   ├── service.yaml
│   └── servicemonitor.yaml            # ServiceMonitor CRD
├── grafana/
│   ├── deployment.yaml
│   ├── service.yaml
│   └── ingress.yaml
├── jaeger/
│   ├── deployment.yaml
│   └── service.yaml
├── victoriametrics/
│   └── statefulset.yaml
└── exporters/
    ├── postgres-exporter.yaml
    ├── redis-exporter.yaml
    └── rabbitmq-exporter.yaml
```

### 8.3 Dockerfile（Rust 多阶段构建）

```dockerfile
# 阶段一：编译
FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release --bin monitor-api

# 阶段二：运行
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/monitor-api .
COPY configs/ ./configs/
EXPOSE 9100 9101
ENTRYPOINT ["./monitor-api"]
```

---

## 九、安全与性能设计

### 9.1 安全防护

| 防护项               | 措施                                                         |
| -------------------- | ------------------------------------------------------------ |
| **端点保护**         | `/metrics`、`/debug/pprof/` 配置 IP 白名单或 Basic Auth      |
| **敏感数据脱敏**     | 追踪 Span 和日志中绝不包含密码、Token、患者隐私信息（姓名、身份证号等） |
| **TLS 加密**         | 所有对外通信启用 TLS (rustls)，内部服务通信启用 mTLS         |
| **API 鉴权**         | monitor-api 启用 JWT + API Key 双重认证，RBAC 控制只读/管理员 |
| **审计日志**         | 告警操作（确认/静默）、配置变更记录写入 PostgreSQL 审计表，不可删除 |
| **供应链安全**       | `cargo audit` 检查依赖漏洞，`cargo deny` 管理许可证合规     |
| **内存安全**         | Rust 类型系统在编译期消除 use-after-free、double-free、数据竞争等内存问题 |

### 9.2 性能开销控制

| 信号类型   | 目标开销控制           | 实现方式                                              |
| ---------- | ---------------------- | ----------------------------------------------------- |
| 采集开销   | 单节点支撑 500+ target | Tokio 多线程 + 连接池复用 + 非阻塞 I/O                |
| 处理开销   | < 2% CPU               | 预聚合减少实时计算，批量写入减少存储 I/O              |
| 内存占用   | < 512MB (单节点)        | 指标数据流式处理，避免全量加载到内存                  |
| 存储写入   | 可控写入放大           | 采集层预聚合 + 降采样策略 + VictoriaMetrics 高压缩比  |

- **连接池**：HTTP 客户端 (reqwest) 开启连接池，gRPC 客户端 (Tonic) 维持长连接
- **批量写入**：指标数据以 1000 条/批写入 VictoriaMetrics，Trace 数据以 100 Span/批写入 Jaeger
- **锁竞争**：优先使用 channel 和 actor 模型（Tokio mpsc）代替 Mutex
- **零拷贝**：利用 Rust 的 `Bytes`、`&str` 等零拷贝类型，减少内存分配

---

## 十、Grafana 仪表盘规划

### 10.1 核心仪表盘列表

| 仪表盘名称                     | 数据源                   | 内容                                         |
| ------------------------------ | ------------------------ | -------------------------------------------- |
| **HIS 业务总览**               | Prometheus + PostgreSQL  | 实时挂号量/就诊量/处方量/收费金额/住院人数   |
| **微服务运行状态**             | Prometheus               | 18 服务 CPU/Mem/Goroutine/gRPC P95/错误率     |
| **基础设施大盘**               | Prometheus               | DB 连接池/Redis 命中率/RabbitMQ 积压/Nginx   |
| **业务链路大盘**               | Prometheus               | 挂号→就诊→处方→收费→发药端到端延迟与成功率   |
| **慢查询大盘**                 | Prometheus               | 17 个 DB 的慢查询 TOP N，关联到具体服务和接口 |
| **SLO 合规大盘**               | Prometheus               | 按服务/接口维度的 SLI vs SLO 差距            |
| **告警总览**                   | monitor-api REST         | 活跃告警、恢复趋势、告警处理时长统计         |
| **探活状态**                   | Prometheus               | monitor-probe 黑盒探活结果与延迟分布         |

### 10.2 仪表盘嵌入

通过 Grafana 的 `iframe` 嵌入能力或 `auth proxy` 模式，将上述仪表盘嵌入 HIS-Go 运营管理后台（`his-web-admin`），实现前台业务与后台监控的统一界面。

---

## 十一、实施路线图

### 第一阶段：核心框架搭建（2-3 周）

- 搭建 Cargo Workspace，完成所有 crate 骨架与依赖声明
- 实现 `monitor-core`：配置加载 (config-rs + Nacos)、错误类型、数据模型、Proto 定义
- 实现 `monitor-api`：Axum 服务器、健康检查、`/metrics` 端点、中间件链
- 实现 `monitor-probe`：基础 HTTP 探活能力，验证 Gateway 连通性
- 集成 Docker 构建流水线

### 第二阶段：采集与处理（2-3 周）

- 实现 `monitor-collector`：Prometheus scraper、基础设施 Exporter（PG/Redis/RabbitMQ）
- 实现 `monitor-processor`：指标聚合降采样、告警规则评估引擎
- 对接 VictoriaMetrics / Jaeger 存储后端
- 配置 Prometheus + Grafana 数据源，搭建第一期仪表盘

### 第三阶段：告警与通知（1-2 周）

- 实现 `monitor-alert`：告警生命周期管理、静默窗口、告警抑制
- 接入多通道通知（钉钉、企业微信、邮件、短信）
- 配置 HIS-Go 核心告警规则（服务存活、错误率、DB 连接池等）
- 编写告警操作 Runbook

### 第四阶段：精细化运营（1-2 周）

- 完善 `monitor-probe`：全部 5 个核心业务链路探活场景
- 实现异常检测模块（基线学习、同环比波动检测）
- 搭建 HIS 业务特有指标大盘和 SLO 合规大盘
- `his-web-admin` 运营管理后台嵌入监控面板
- 全链路压测验证监控系统自身性能

---

## 十二、总结

HIS-Monitor-rs 作为 HIS-Go 的 Rust 监控后端，围绕 **指标 (Metrics) → 追踪 (Tracing) → 日志 (Logging) → 探活 (Probe)** 四大信号，构建了完整的可观测性体系。核心要点：

1. **Rust 高性能基础**：利用 Tokio 异步运行时、零成本抽象和内存安全特性，构建低开销、高可靠性的监控系统
2. **标准协议兼容**：全链路采用 OpenTelemetry + Prometheus 标准，与 HIS-Go 及 Grafana/Jaeger/Alertmanager 生态无缝对接
3. **分层解耦与独立扩展**：采集层 → 处理层 → 存储层 → API 层 → 告警层 → 探活层，各层通过 Cargo Workspace 独立演进
4. **业务导向的监控**：在系统指标之上，构建 HIS 业务特有指标和全链路合成监控，精准反映医疗业务健康度
5. **生产就绪**：支持 Docker Compose 和 K8s 双部署模式，与 HIS-Go 现有部署方案无缝兼容
6. **安全合规**：全面考虑医疗数据脱敏、端点保护、TLS 加密和审计日志要求
