# PulseOn Native 版本架构设计

> 版本：v2.0（含 AI Native 扩展）
> 日期：2026-06-22
> 定位：训练指标追踪产品的 Native（本地优先）版本架构。为未来 Cloud 版本预留层替换能力，不重写系统。SDK 不仅服务于开发者和训练者，更将 AI Agent 作为一等用户，支持 auto research 工作流。
> 参考：`docs/reference/training-metrics-hosting-boundaries.md`（托管边界思想）、`docs/reference/training-metrics-storage-architecture.md`（存储架构决策）、`docs/reference/ducklake-archive.md`（DuckLake 验证存档）

---

## 目录

### Part I：基础架构

1. [设计哲学与分层原则](#1-设计哲学与分层原则)
2. [三层 Trait 架构（核心）](#2-三层-trait-架构核心)
3. [逻辑数据模型](#3-逻辑数据模型)
4. [写入路径：Staging → Flush](#4-写入路径staging--flush)
5. [查询路径](#5-查询路径)
6. [Rust Crate/模块结构](#6-rust-crate模块结构)
7. [Rust vs Python 职责边界](#7-rust-vs-python-职责边界)
8. [Python SDK API 表面](#8-python-sdk-api-表面)
9. [配置与部署模式](#9-配置与部署模式)
10. [关键风险、权衡与开放问题](#10-关键风险权衡与开放问题)

### Part II：AI Native 扩展

11. [AI Native 设计哲学](#11-ai-native-设计哲学)
12. [数据模型扩展（Auto Research）](#12-数据模型扩展auto-research)
13. [语义查询层（AgentToolInterface trait）](#13-语义查询层agenttoolinterface-trait)
14. [LLM 友好的输出格式与渐进式披露](#14-llm-友好的输出格式与渐进式披露)
15. [Tool-Calling 接口（LLM function calling）](#15-tool-calling-接口llm-function-calling)
16. [MCP Server（Agent 的通用接口）](#16-mcp-serveragent-的通用接口)
17. [Auto-Research 工作流设计](#17-auto-research-工作流设计)
18. [实时监控（Agent 视角）](#18-实时监控agent-视角)
19. [Python SDK：agent 模块](#19-python-sdkagent-模块)
20. [Rust 模块扩展](#20-rust-模块扩展)
21. [降采样策略（上下文效率）](#21-降采样策略上下文效率)
22. [AI Native 与现有架构的关系](#22-ai-native-与现有架构的关系)

---

## 1. 设计哲学与分层原则

### 1.1 核心判断：产品卖的不是数据库，而是托管边界

MotherDuck 的 `catalog / storage / compute` 三档设计本质上在定义**控制权归属**——不是"我的服务提供哪些功能"，而是"哪一层由谁托管、谁兜底、谁收费"。训练指标产品天然面对本地、云端、私有化三种落点，这个三档模型恰好映射到我们的架构：

| 层 | 职责 | Native 版谁托管 | 未来 Cloud 版谁托管 |
|---|---|---|---|
| **Catalog** | run、metric、file、snapshot 元数据管理 | 用户本地（DuckLake + SQLite） | 平台托管（PostgreSQL） |
| **Storage** | Parquet + zstd 事实数据文件 | 用户指定（本地磁盘 或 S3 兼容对象存储） | 平台托管或用户自带 |
| **Compute** | 查询执行引擎 | 用户本地（DuckDB embedded） | 平台托管（ClickHouse） |

**设计原则**：先把控制权切成层，再决定每层由谁托管。Native 和 Cloud 不是两套系统，而是同一套分层架构的两种托管组合。

### 1.2 统一的是数据协议，不是执行引擎

这是整份架构最重要的判断，直接继承自 reference doc：

- **统一逻辑模型**：run、metric_definition、metric_point、run_event、run_summary 等核心对象在各部署形态下含义一致。
- **统一交换格式**：Parquet + zstd，跨 DuckDB 和 ClickHouse 均可读，用户始终持有开放格式。
- **统一 catalog 模型**：元数据表结构一致（至少逻辑上），只是底层数据库不同（SQLite → PostgreSQL）。
- **执行引擎可变**：Native 用 DuckDB，Cloud 用 ClickHouse，未来可扩展其他引擎。

### 1.3 DuckLake 的角色定位（关键调和）

Reference doc 明确指出"不直接依赖 DuckLake 作为底层协议"，给出三条理由：ClickHouse 兼容性、专用 catalog 比通用 lakehouse 更简单、核心判断可以自己实现。这些理由在 Cloud 版本中完全成立。

**但 Native 版本不同**。Native 版本的用户场景是"单机、单用户、零守护进程"，DuckLake 在这个约束下提供了工程上最成熟的开箱方案：

- 内联数据自动管理（小写入不产生小文件）
- 自动 flush 到 Parquet
- Hive 分区目录结构
- SQLite/PostgreSQL 双 catalog 后端（用户无需维护数据库服务）
- S3 兼容对象存储支持

**调和方案：DuckLake 作为 catalog 机制，不作为产品协议。**

```
┌─────────────────────────────────────────────┐
│  PulseOn 产品层                              │
│  - 逻辑模型（Run, Metric, Event, Summary）   │
│  - 统一查询接口（listRuns, queryMetricSeries）│
│  - 三层 trait 抽象（Catalog/Storage/Compute） │
│  - Python SDK                                │
├─────────────────────────────────────────────┤
│  Native 版实现层                             │
│  - CatalogLayer  →  DuckLake + SQLite        │
│  - StorageLayer  →  DuckLake data_path       │
│  - ComputeLayer  →  DuckDB                   │
├─────────────────────────────────────────────┤
│  可替换为 Cloud 版实现层                      │
│  - CatalogLayer  →  PostgreSQL (direct)      │
│  - StorageLayer  →  Object Storage SDK       │
│  - ComputeLayer  →  ClickHouse HTTP          │
└─────────────────────────────────────────────┘
```

**DuckLake 管什么**：文件注册（`ducklake_data_file`）、快照提交（`ducklake_snapshot`）、schema 版本管理（`ducklake_schema_versions`）、分区定义（`ducklake_partition_column`）、内联数据管理（`ducklake_inlined_data_*`）、Parquet flush/compact（`CHECKPOINT`/`ducklake_flush_inlined_data`）。

**PulseOn 管什么**：逻辑模型定义（`runs`/`metric_definitions`/`metric_points` 等业务表）、统一查询接口、三层 trait 抽象、写入/查询编排、Python SDK。

**这是"DuckLake inside"而不是"DuckLake as protocol"**：DuckLake 是 Native 版 catalog 层的实现细节，被 `CatalogLayer` trait 完全封装。Cloud 版换 PostgreSQL 时，DuckLake 可以完全移除，不留痕迹。

---

## 2. 三层 Trait 架构（核心）

### 2.1 架构总览

```
┌──────────────────────────────────────────────────┐
│                   engine::Client                   │
│  (orchestrates Catalog + Storage + Compute)        │
├──────────────────────────────────────────────────┤
│   CatalogLayer       StorageLayer    ComputeLayer  │
│   (trait)            (trait)         (trait)       │
├──────────────────┬──────────────────┬─────────────┤
│ DuckLakeSqlite   │ DuckLakeStorage  │ DuckDBCompute│  ← Native
│ Catalog          │ (via data_path)  │              │
├──────────────────┼──────────────────┼─────────────┤
│ PostgresCatalog  │ S3ObjectStorage  │ ClickHouse   │  ← Cloud (sketch)
│                  │ LocalFileStorage │ Compute      │
└──────────────────┴──────────────────┴─────────────┘
```

**关键洞察**：Native 版本中 DuckLake 同时承载 catalog 和 storage 的实现（通过 DuckDB 连接操作 DuckLake 管理的表和数据文件），但 trait 级别严格分离。DuckDB 连接在 Native 版本中被 catalog 和 compute 共享（同一个 `duckdb::Connection`），但这不影响 trait 的独立性和可替换性。

### 2.2 `CatalogLayer` trait

Catalog 层管理所有元数据：runs、projects、metric_definitions、tags、configs、artifacts、run_summaries，以及 DuckLake 级别的 files/snapshots（当实现是 DuckLake 时）。

```rust
use std::sync::Arc;
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, Utc};
use crate::model::*;

/// 目录层抽象：管理所有元数据对象。
/// Native 实现：DuckLake + SQLite（通过 DuckDB 连接操作）
/// Cloud 实现：PostgreSQL（通过 sqlx 直接连接）
#[async_trait::async_trait]
pub trait CatalogLayer: Send + Sync {
    // ── Workspace ──────────────────────────────────────

    async fn create_workspace(&self, name: &str) -> Result<Workspace>;
    async fn get_workspace(&self, id: &str) -> Result<Workspace>;
    async fn list_workspaces(&self) -> Result<Vec<Workspace>>;

    // ── Project ────────────────────────────────────────

    async fn create_project(&self, workspace_id: &str, name: &str) -> Result<Project>;
    async fn get_project(&self, id: &str) -> Result<Project>;
    async fn list_projects(&self, workspace_id: &str) -> Result<Vec<Project>>;

    // ── Run ────────────────────────────────────────────

    async fn create_run(
        &self,
        project_id: &str,
        name: &str,
        config: Option<serde_json::Value>,
        tags: &[String],
    ) -> Result<Run>;
    async fn get_run(&self, run_id: &str) -> Result<Run>;
    async fn list_runs(
        &self,
        project_id: &str,
        filters: Option<RunFilter>,
    ) -> Result<Vec<Run>>;
    async fn update_run_status(&self, run_id: &str, status: RunStatus) -> Result<()>;
    async fn finish_run(&self, run_id: &str) -> Result<()>;

    // ── Metric Definition ──────────────────────────────

    async fn register_metric(
        &self,
        run_id: &str,
        metric_name: &str,
        value_type: ValueType,
        phase: Option<&str>,
    ) -> Result<MetricDefinition>;
    async fn list_metrics(&self, run_id: &str) -> Result<Vec<MetricDefinition>>;

    // ── Run Summary ────────────────────────────────────

    async fn upsert_run_summary(&self, run_id: &str, summary: RunSummary) -> Result<()>;
    async fn get_run_summary(&self, run_id: &str) -> Result<Option<RunSummary>>;
    async fn query_run_summaries(
        &self,
        project_id: &str,
        filters: Option<SummaryFilter>,
        sort: Option<SummarySort>,
    ) -> Result<Vec<RunSummaryRow>>;

    // ── Tag / Config / Artifact ────────────────────────

    async fn add_tags(&self, run_id: &str, tags: &[String]) -> Result<()>;
    async fn save_config(&self, run_id: &str, config: serde_json::Value) -> Result<()>;
    async fn register_artifact(
        &self,
        run_id: &str,
        name: &str,
        artifact_type: &str,
        path: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<Artifact>;

    // ── Lifecycle ──────────────────────────────────────

    /// 初始化 catalog（创建系统表 / ducklake ATTACH 等）
    async fn initialize(&self) -> Result<()>;
    /// 关闭 catalog 连接，执行最终 checkpoint/flush
    async fn shutdown(&self) -> Result<()>;
}
```

**Native 实现 `DuckLakeSqliteCatalog`**：
- 内部持有 `Arc<duckdb::Connection>`（DuckDB 进程内连接）
- `initialize()` 执行：`INSTALL ducklake; LOAD ducklake; ATTACH '<path>.sqlite' AS dl (TYPE ducklake, CATALOG 'sqlite', DATA_PATH '<datapath>')`
- 业务表（`runs`、`metric_definitions` 等）创建为 DuckLake 表（`CREATE TABLE dl.runs (...)`），利用 DuckLake 的 snapshot/file 管理。
- `metric_points` 表为 DuckLake 表，分区键 `(project_id, run_id, metric_name)`（identity partition），让 DuckLake 自动按 hive 目录组织 Parquet 文件。
- `create_run` / `register_metric` 等通过 DuckDB SQL 执行 `INSERT INTO dl.<table> VALUES (...)`。
- `list_runs` / `query_run_summaries` 通过 DuckDB SQL 执行 `SELECT ... FROM dl.runs WHERE ...`。
- `shutdown()` 执行 `CHECKPOINT dl`（flush 内联数据），然后 `DETACH dl`。

**Cloud 实现 `PostgresCatalog`（sketch）**：
- 内部持有 `sqlx::PgPool`
- 所有业务表为普通 PG 表（无 DuckLake 依赖）
- `metric_points` 不存在于 catalog 中——Cloud 版的 metric_points 数据以 Parquet 文件形式存在于对象存储中，catalog 只记录文件注册信息（`files` 表记录每个 Parquet 文件的路径、行数、step 范围等）
- `shutdown()` 为 no-op

### 2.3 `StorageLayer` trait

Storage 层管理 Parquet 事实数据文件的读写。在 Native 版本中，常规写入路径的 I/O 由 DuckLake 内部处理（通过 `data_path`），但 trait 仍然独立存在以确保 Cloud 版的可替换性，并用于导出/迁移等场景。

```rust
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, Utc};

/// 存储层抽象：管理 Parquet 数据文件的读写。
/// Native 实现：委托给 DuckLake（通过 data_path 配置）
///            + 独立实现用于直接读写 Parquet（跨版本迁移/导出）
/// Cloud 实现：object_store crate（S3/GCS/Azure/本地）
#[async_trait::async_trait]
pub trait StorageLayer: Send + Sync {
    /// 写入一批 Arrow RecordBatch 到指定路径的 Parquet 文件
    async fn write_parquet(
        &self,
        path: &str,            // e.g. "metric_points/project=X/run=Y/metric=loss/part-001.parquet"
        batches: &[RecordBatch],
    ) -> Result<FileInfo>;

    /// 读取指定路径的 Parquet 文件，返回 Arrow RecordBatch
    async fn read_parquet(
        &self,
        path: &str,
        columns: Option<&[String]>,
        row_filter: Option<&str>,   // DuckDB/Arrow predicate pushdown
    ) -> Result<Vec<RecordBatch>>;

    /// 列出指定前缀下的所有文件
    async fn list_files(&self, prefix: &str) -> Result<Vec<FileInfo>>;

    /// 删除文件
    async fn delete_file(&self, path: &str) -> Result<()>;

    /// 检查文件是否存在
    async fn file_exists(&self, path: &str) -> Result<bool>;

    /// 获取文件元信息
    async fn file_info(&self, path: &str) -> Result<FileInfo>;
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: String,
    pub size_bytes: u64,
    pub row_count: Option<u64>,
    pub created_at: DateTime<Utc>,
}
```

**Native 实现 `DuckLakeStorage`**：
- Native 版本的常规写入路径**不直接调用 StorageLayer**。数据通过 DuckLake 表的 `INSERT` 写入，DuckLake 自动管理内联和 flush 到 Parquet。
- `read_parquet` / `list_files` / `delete_file` 等方法在 Native 版本中保留用于：跨版本数据导出、手动 compact、迁移工具等。
- 对于 `data_path` 为 S3 的情况，这些操作通过 DuckDB 的 `httpfs` 扩展完成（DuckDB 已内置支持）。

**Cloud 实现 `S3ObjectStorage`（sketch）**：
- 使用 `object_store` crate（Apache Arrow 生态的通用对象存储抽象，支持 S3/GCS/Azure/本地，5.2M 月下载量，1251 个 crate 依赖）
- Parquet 文件的读写走对象存储 SDK
- Cloud 版的 ingest 服务调用 `write_parquet` 将 staging buffer 批量落盘，然后 `CatalogLayer` 注册文件信息

### 2.4 `ComputeLayer` trait

Compute 层执行分析查询，返回 Arrow RecordBatch。Native 版本用 DuckDB 内存引擎，Cloud 版本用 ClickHouse。

```rust
/// 计算层抽象：执行分析查询。
/// Native 实现：DuckDB embedded（直接查 DuckLake 表，读取 Parquet）
/// Cloud 实现：ClickHouse（通过 HTTP/tcp 查询 serving 表）
#[async_trait::async_trait]
pub trait ComputeLayer: Send + Sync {
    /// 执行查询，返回 Arrow RecordBatch
    async fn execute(&self, query: &str) -> Result<Vec<RecordBatch>>;

    /// 注册 catalog（仅 DuckDB 需要；ClickHouse 实现为 no-op）
    async fn register_catalog(&self, catalog: &dyn CatalogLayer) -> Result<()>;

    /// 根据统一查询接口方法执行，返回 Arrow 结果
    /// （高层封装，内部转换为引擎特定 SQL）
    async fn query_metric_series(
        &self,
        run_ids: &[String],
        metric_names: &[String],
        step_range: Option<(i64, i64)>,
        timestamp_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
        phase: Option<&str>,
        limit: Option<u64>,
    ) -> Result<Vec<RecordBatch>>;

    async fn query_run_summary(&self, run_ids: &[String]) -> Result<Vec<RecordBatch>>;

    async fn list_run_events(
        &self,
        run_id: &str,
        event_type: Option<&str>,
        timestamp_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    ) -> Result<Vec<RecordBatch>>;
}
```

**Native 实现 `DuckDBCompute`**：
- 内部持有 `Arc<duckdb::Connection>`（与 `DuckLakeSqliteCatalog` 共享同一个 connection）
- `register_catalog()` ATTACH DuckLake（如果尚未 ATTACH）
- `query_metric_series()` 生成 SQL：`SELECT * FROM dl.metric_points WHERE run_id IN (...) AND metric_name IN (...) ORDER BY step`
- DuckDB 通过 DuckLake 的 Parquet 扫描执行查询，利用分区裁剪（project_id/run_id/metric_name）和列裁剪优化
- 结果以 Arrow RecordBatch 返回（duckdb-rs 的 `query_arrow()` / `stream_arrow()` API）

**Cloud 实现 `ClickHouseCompute`（sketch）**：
- 内部持有 HTTP client（或 tcp client via `clickhouse-rs`）
- `register_catalog()` 为 no-op
- `query_metric_series()` 转换为 ClickHouse SQL 并通过 HTTP 查询
- 结果以 Arrow RecordBatch（通过 ClickHouse 的 Arrow 输出格式）返回

### 2.5 关键耦合分析：DuckLake + DuckDB 运行时绑定 vs 层可替换性

**问题**：Native 版本中 DuckLake 作为 DuckDB 扩展运行，所以 catalog 操作（CREATE TABLE, INSERT）和 compute 操作（SELECT）都经过同一个 DuckDB 进程。这是否破坏了层独立性？

**答案：不破坏。** 原因如下：

1. **Trait 级别完全解耦**。`DuckLakeSqliteCatalog` 和 `DuckDBCompute` 是两个独立的 struct，各自实现各自的 trait。它们可以各自独立地被 mock/testing 实现替换。

2. **共享的是 DuckDB 连接，不是耦合**。两个 impl 共享同一个 `Arc<duckdb::Connection>` 是 Native 版本的实现优化，不是架构约束。如果未来需要一个不共享连接的方案（如 catalog 远程 PG + compute 本地 DuckDB），只需修改 `engine::Client` 的初始化逻辑，不需要修改任何 trait 定义。

3. **Cloud 路径完全独立**。`PostgresCatalog` 不需要 DuckDB，`ClickHouseCompute` 不需要 DuckDB。Native 版本的耦合不会传染到 Cloud 版本。

4. **Parquet 是桥梁**。DuckLake flush 出的 Parquet 文件是标准格式。即使 DuckLake 和 DuckDB 在 Native 版本紧密耦合，只要 Parquet 文件本身是标准的，Cloud 版的 ClickHouse 就能直接读取。**数据格式的开放性是层可替换性的真正支撑。**

---

## 3. 逻辑数据模型

### 3.1 核心实体关系

```
Workspace 1──N Project 1──N Run
                                  │
                  ┌───────────────┼────────────────┐
                  │               │                │
            MetricDefinition  metric_point    RunEvent
            (1 metric/run)   (N points/run)  (N events/run)
                                  │
                            RunSummary
                            (1 per run, updatable)
```

### 3.2 表设计：元信息表 vs 事实表

**元信息表（数据量小，更新频繁，需事务一致性）—— 存在 catalog 中**：

| 表名 | 存储位置 | 说明 |
|------|---------|------|
| `workspaces` | DuckLake table（catalog） | 工作空间 |
| `projects` | DuckLake table（catalog） | 项目 |
| `runs` | DuckLake table（catalog） | 训练运行 |
| `metric_definitions` | DuckLake table（catalog） | 指标定义（名称、类型、phase） |
| `tags` | DuckLake table（catalog） | 标签 |
| `run_tags` | DuckLake table（catalog） | run-tag 多对多 |
| `configs` | DuckLake table（catalog） | run 配置（JSON） |
| `artifacts` | DuckLake table（catalog） | 产物索引 |
| `run_summary` | DuckLake table（catalog） | run 汇总（upsertable） |

**事实表（数据量大，append-only，适合列式扫描）—— DuckLake 管理的 Parquet 数据文件**：

| 表名 | 存储位置 | 分区键 | 说明 |
|------|---------|--------|------|
| `metric_points` | DuckLake table → Parquet | `(project_id, run_id, metric_name)` | 指标时序数据（长表） |
| `run_events` | DuckLake table → Parquet | `(project_id, run_id)` | 运行事件流 |

**关键设计决策**：`run_summary` 放在 catalog 中（而非 Parquet 事实表），因为它是单行 upsertable（每个 run 一条），数据量极小，查询模式是点查/列表排序，适合 SQLite/PostgreSQL 索引。

### 3.3 核心表 Schema

#### `runs`（catalog 表）

```sql
CREATE TABLE runs (
    id           VARCHAR PRIMARY KEY,       -- UUID
    project_id   VARCHAR NOT NULL,
    workspace_id VARCHAR NOT NULL,
    name         VARCHAR NOT NULL,
    display_name VARCHAR,
    status       VARCHAR NOT NULL DEFAULT 'running',  -- running, finished, failed, crashed
    started_at   TIMESTAMP NOT NULL DEFAULT now(),
    finished_at  TIMESTAMP,
    notes        VARCHAR,
    host         VARCHAR,                   -- hostname
    pid          BIGINT,
    created_at   TIMESTAMP NOT NULL DEFAULT now(),
    updated_at   TIMESTAMP NOT NULL DEFAULT now()
);
```

#### `metric_definitions`（catalog 表）

```sql
CREATE TABLE metric_definitions (
    id           VARCHAR PRIMARY KEY,       -- UUID
    run_id       VARCHAR NOT NULL,
    project_id   VARCHAR NOT NULL,
    workspace_id VARCHAR NOT NULL,
    name         VARCHAR NOT NULL,          -- metric name, e.g. "train/loss"
    value_type   VARCHAR NOT NULL,          -- 'f64', 'i64', 'str', 'bool'
    phase        VARCHAR,                   -- 'train', 'val', 'test', etc.
    created_at   TIMESTAMP NOT NULL DEFAULT now(),
    UNIQUE(run_id, name)                    -- one definition per metric per run
);
```

#### `metric_points`（事实表，DuckLake managed → Parquet）

```sql
CREATE TABLE metric_points (
    workspace_id VARCHAR NOT NULL,
    project_id   VARCHAR NOT NULL,
    run_id       VARCHAR NOT NULL,
    metric_name  VARCHAR NOT NULL,
    step         BIGINT NOT NULL,          -- training step
    timestamp    TIMESTAMP NOT NULL,       -- wall clock time
    value_f64    DOUBLE,
    value_i64    BIGINT,
    value_str    VARCHAR,
    value_bool   BOOLEAN,
    value_type   VARCHAR NOT NULL,         -- discriminator: which column is valid
    phase        VARCHAR,                  -- train/val/test
    rank         INTEGER,                  -- for distributed training
    source       VARCHAR                   -- 'sdk', 'import', etc.
);
-- DuckLake 分区语法（创建表后设置）：
-- ALTER TABLE dl.metric_points SET PARTITIONED BY (project_id, run_id, metric_name);
```

**为什么是长表而不是宽表**：
- 用户自定义 metric 数量不固定，宽表需要动态 ALTER TABLE ADD COLUMN
- 稀疏记录（某些 step 只记录部分 metric）在长表中自然处理
- DuckDB/ClickHouse 的列裁剪在长表上效果不差（`WHERE metric_name='loss'` 配合分区裁剪）
- 前端折线图的热路径 = `run_id + metric_name + step + value`，长表不需要解析 JSON

**为什么分 4 个 value 列（`value_f64/i64/str/bool`）而不是 JSON blob**：
- DuckDB 对 typed column 的 SIMD 加速远好于 JSON 解析
- ClickHouse 导入后可直接映射为稳定 schema
- 每次查询只需要其中一个 value 列，`value_type` 字段指示有效列
- 这是 DuckDB/DuckLake 验证中确认的最佳实践

#### `run_events`（事实表，DuckLake managed → Parquet）

```sql
CREATE TABLE run_events (
    id           VARCHAR PRIMARY KEY,
    workspace_id VARCHAR NOT NULL,
    project_id   VARCHAR NOT NULL,
    run_id       VARCHAR NOT NULL,
    event_type   VARCHAR NOT NULL,         -- 'checkpoint', 'alert', 'crash', 'user_note'
    message      VARCHAR,
    metadata     VARCHAR,                  -- JSON string for extensibility
    timestamp    TIMESTAMP NOT NULL
);
-- ALTER TABLE dl.run_events SET PARTITIONED BY (project_id, run_id);
```

#### `run_summary`（catalog 表，upsertable）

```sql
CREATE TABLE run_summary (
    run_id         VARCHAR PRIMARY KEY,
    project_id     VARCHAR NOT NULL,
    workspace_id   VARCHAR NOT NULL,
    metrics_summary VARCHAR,               -- JSON: {"loss": {"min": 0.1, "max": 2.3, "last": 0.5, "best": 0.1}, ...}
    total_steps    BIGINT,
    duration_seconds DOUBLE,
    updated_at     TIMESTAMP NOT NULL DEFAULT now()
);
```

### 3.4 物理目录结构（DuckLake auto-managed）

DuckLake 根据 `metric_points` 的分区键 `(project_id, run_id, metric_name)` 自动生成 hive 风格目录：

```
<data_path>/
  main/
    metric_points/
      project_id=<project-uuid>/
        run_id=<run-uuid>/
          metric_name=train~loss/           # DuckLake 对特殊字符有处理
            ducklake-<uuid>.parquet
          metric_name=train~accuracy/
            ducklake-<uuid>.parquet
    run_events/
      project_id=<project-uuid>/
        run_id=<run-uuid>/
          ducklake-<uuid>.parquet
```

DuckLake 的 UUID 命名文件由 DuckLake 内部管理，PulseOn 产品层**不干预**文件命名。如需导出/迁移，通过 `StorageLayer` trait 的 `read_parquet` 读取后重写为标准命名 Parquet。

### 3.5 表与 DuckLake 模型的映射

| 产品概念 | DuckLake 概念 | 说明 |
|---------|--------------|------|
| 业务表（`runs`, `metric_definitions` 等） | DuckLake table | 通过 `CREATE TABLE dl.<table>` 创建 |
| 数据行 | DuckLake 内联数据 或 Parquet 数据文件中的 row | 小批量写入进内联，大批量 flush 进 Parquet |
| 文件注册 | `ducklake_data_file` 记录 | DuckLake 自动管理 |
| 版本/快照 | `ducklake_snapshot` + `ducklake_snapshot_changes` | 每次 INSERT/CHECKPOINT 产生新快照 |
| Schema 变更 | `ducklake_schema_versions` | ALTER TABLE 后自动记录 |
| 分区定义 | `ducklake_partition_column` | 通过 `ALTER TABLE SET PARTITIONED BY` 设置 |

---

## 4. 写入路径：Staging → Flush

### 4.1 核心策略：使用 DuckLake 内置 inlining 作为 staging 机制

Reference doc 描述了"小写入先进入 staging，再批量 flush 成 Parquet"的通用策略，并建议自建 staging 层。但 DuckLake 的内联数据（Data Inlining）机制**在 Native 版本中恰好就是 staging 层的最优实现**：

| 对比维度 | 自建 staging（SQLite WAL + 手动 flush） | DuckLake 内置 inlining |
|---------|---------------------------------------|----------------------|
| 实现复杂度 | 需维护独立的 staging 表 + flush 逻辑 + Parquet writer | 零额外代码，DuckLake 自动管理 |
| 小文件问题 | 需要手动控制 flush 阈值和合并 | 内联数据存 catalog，flush 时一次写 Parquet |
| 分区感知 | 需手动按分区 flush 到正确目录 | DuckLake 自动按分区键写入 hive 目录 |
| 数据可见性 | staging 期间数据对查询不可见（除非特殊处理） | 内联数据对 DuckDB SELECT 天然可见 |
| 崩溃恢复 | 需要 WAL checkpoint 逻辑 | DuckLake snapshot 原子提交 |
| 跨引擎 | 自建可以控制 flush 粒度 | DuckLake 不支持分区级 flush |

**结论**：对于 Native 版本的约束（单 writer、单机、app-only、零守护进程），DuckLake 内置 inlining 是明显更优的选择。唯一的不足（不支持分区级 flush）在 run-centric 场景下不构成实际问题——flush 总是在 run 结束时全表执行。

### 4.2 写入流程

```
Python SDK                    Rust Core (engine)                  DuckLake/SQLite
─────────                     ────────────────                    ───────────────

log_metric("loss", step, val)
  │
  ├─► PyO3 call
  │   run.log_metric(name, value)
  │     │
  │     ├─► 验证 metric_definition 是否已注册
  │     │   （首次出现会自动注册）
  │     │
  │     └─► INSERT INTO dl.metric_points VALUES (...)
  │          │
  │          │  DuckLake 判断:
  │          │  - 行数 < DATA_INLINING_ROW_LIMIT (默认500)?
  │          │    ├─ YES: 写入内联数据表（catalog 内）  ◄────────
  │          │    └─ NO:  直接写入 Parquet 文件          ◄────────
  │          │           （按分区键 hive 目录结构）
  │          ▼
  │   返回成功
  │
run.finish()
  │
  ├─► PyO3 call
  │   run.finish()
  │     │
  │     ├─► catalog.finish_run(run_id)  # 更新 status=finished
  │     ├─► 计算并 upsert run_summary
  │     └─► flush_inlined_data()
  │          │
  │          └─► CALL ducklake_flush_inlined_data('dl')
  │               │
  │               ├─ 读取内联数据表所有行
  │               ├─ 按分区键分组
  │               ├─ 写入 Parquet 到对应 hive 目录
  │               ├─ 注册到 ducklake_data_file
  │               └─ 创建新 snapshot
  │
run.log_metrics({"loss": 1.2,         # batch write
                 "acc": 0.5}, step)
  │
  └─► 批量 INSERT（单条 SQL 多 VALUES 或 prepared statement）
      减少 DuckDB 调用次数，提高吞吐
```

### 4.3 Flush 触发规则

| 触发条件 | 行为 | 实现方式 |
|---------|------|---------|
| **内联数据自动 flush** | 累积行数 ≥ `DATA_INLINING_ROW_LIMIT`（默认 500）时触发 | DuckLake 内置，无需代码 |
| **Run 结束时强制 flush** | `finish_run()` → `CALL ducklake_flush_inlined_data('dl')` | 应用层调用 |
| **定期自动 flush（可选）** | 每 N 秒或每 M 个 step 检查一次内联数据量 | 应用层定时器（future proof） |
| **Compact** | 多个小 Parquet 合并为大文件（目标 64-256MB） | 调用 `ducklake_merge_adjacent_files` |

**DuckLake 配置**：

```sql
-- ATTACH 时设置内联阈值（推荐 500-1000 行，平衡内联查询速度与 flush 频率）
ATTACH 'catalog.sqlite' AS dl (
    TYPE ducklake,
    CATALOG 'sqlite',
    DATA_PATH './data/',
    DATA_INLINING_ROW_LIMIT 500
);
```

**为什么阈值设为 500 而不是 DuckLake 默认的 10**：
- 训练指标的典型写入频率是 1-10 step/秒，每 step 可能写入 1-20 个 metric
- 阈值 10 意味着几乎每次 INSERT 都可能触发 flush，产生大量小 Parquet
- 阈值 500 意味着每 50-500 个 step 才 flush 一次（取决于 metric 数量），训练过程中的数据在内联表中，查询依然可见
- 阈值过大会增加内存压力和 crash 风险（内联数据在 DuckDB 内存中）

### 4.4 `data_inlining_row_limit = 0` 是禁路

DuckLake 验证中已实测证实：禁用内联（row_limit=0）会导致逐条 INSERT 每次都创建一个独立 Parquet 文件。10000 个 step = 10000 个 1.3KB Parquet 文件，**完全丧失列式读取优势**。

**结论**：始终保持内联开启。不要在应用层"优化"掉这个机制。

### 4.5 Compaction 策略

DuckLake 的 `ducklake_merge_adjacent_files` 可用于 compact，但注意：
- 不支持按分区过滤（全表 compact）
- 建议：run 结束时执行一次 compact（小 run 可跳过）
- 如果 run 写入大量数据（数百万行），DuckLake 会自动产生较大的 Parquet 文件，compact 需求降低

```sql
-- run 结束时可选执行
CALL ducklake_merge_adjacent_files('dl', 'metric_points', max_compacted_files => 20);
```

v1 阶段不需要复杂的 background compaction，run 粒度足够。

---

## 5. 查询路径

### 5.1 查询流程

```
Python SDK                    Rust Core (engine)                  DuckDB / DuckLake
─────────                     ────────────────                    ────────────────

client.query_metric_series(
  run_ids=[...], metrics=["loss"])
  │
  ├─► PyO3 call
  │   compute.query_metric_series(...)
  │     │
  │     └─► SELECT
  │           run_id, metric_name, step,
  │           timestamp, value_f64, phase
  │         FROM dl.metric_points
  │         WHERE run_id IN ('id1', 'id2')
  │           AND metric_name IN ('loss', 'acc')
  │           AND step BETWEEN 0 AND 10000
  │         ORDER BY run_id, step
  │          │
  │          │  DuckDB 执行:
  │          │  - 分区裁剪: 只扫描匹配的 run_id 目录
  │          │  - 列裁剪: 只读取需要的列
  │          │  - Parquet row group pruning
  │          ▼
  │   返回 Arrow RecordBatch[]
  │
  └─► [Arrow → Python: pyarrow Table / polars DF / pandas DF]
```

### 5.2 统一查询接口

以下是 Rust trait 中的统一查询方法（也在 `ComputeLayer` trait 中，但建议单独抽为 `QueryInterface` trait 以解耦查询语义与执行引擎）：

```rust
/// 统一查询接口——前端/CLI/Python SDK 只依赖这组方法，不感知底层引擎。
#[async_trait::async_trait]
pub trait QueryInterface: Send + Sync {
    /// 列出指定 project 下的所有 runs
    async fn list_runs(
        &self,
        project_id: &str,
        filters: Option<RunFilter>,
        sort: Option<RunSort>,
        limit: Option<u32>,
    ) -> Result<Vec<Run>>;

    /// 列出指定 run 的所有 metric 定义
    async fn list_metrics(&self, run_id: &str) -> Result<Vec<MetricDefinition>>;

    /// 查询 metric 时序数据（折线图热路径）
    async fn query_metric_series(
        &self,
        run_ids: &[String],
        metric_names: &[String],
        step_range: Option<(i64, i64)>,
        timestamp_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
        phase: Option<&str>,
        limit: Option<u64>,
    ) -> Result<Vec<RecordBatch>>;

    /// 查询 run 汇总信息（列表页、排序、筛选）
    async fn query_run_summaries(
        &self,
        project_id: &str,
        filters: Option<SummaryFilter>,
        sort: Option<SummarySort>,
        limit: Option<u32>,
    ) -> Result<Vec<RunSummaryRow>>;

    /// 列出 run 事件
    async fn list_run_events(
        &self,
        run_id: &str,
        event_type: Option<&str>,
        timestamp_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
        limit: Option<u32>,
    ) -> Result<Vec<RunEvent>>;

    /// 原始 SQL 查询（高级用户/调试）
    async fn execute_raw(&self, query: &str) -> Result<Vec<RecordBatch>>;
}
```

**Native 实现**：`DuckDBQueryInterface` 持有 `Arc<duckdb::Connection>`，将方法翻译为 DuckDB SQL 并返回 Arrow。

**Cloud 实现**：`ClickHouseQueryInterface` 将方法翻译为 ClickHouse SQL，通过 HTTP/tcp 返回 Arrow。

### 5.3 查询结果返回 Python（零拷贝 Arrow 互操作）

使用 `pyo3-arrow` crate（v0.17+）实现 Rust Arrow ↔ Python PyArrow 的零拷贝传递。`pyo3-arrow` 基于 Arrow PyCapsule Interface（`__arrow_c_stream__` / `__arrow_c_array__`），数据不经过序列化，Python 侧直接访问 Rust 内存中的 Arrow 数据。

**端到端数据流**：

```
DuckDB query → duckdb-rs query_arrow() → arrow::RecordBatch → pyo3-arrow → Python (pyarrow/polars/pandas)
```

**Rust 侧实现**：

```rust
use duckdb::Connection;
use pyo3_arrow::PyRecordBatchReader;

/// 执行 DuckDB 查询，返回 pyarrow RecordBatchReader（零拷贝）
#[pyfunction]
fn query_metric_series(
    py: Python<'_>,
    conn: &Connection,
    run_ids: Vec<String>,
    metrics: Vec<String>,
) -> PyResult<PyRecordBatchReader> {
    // 1. 生成 SQL
    let sql = build_metric_series_sql(&run_ids, &metrics);

    // 2. 通过 duckdb-rs 执行查询，获取 Arrow RecordBatch
    let mut stmt = conn.prepare(&sql)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let batches: Vec<arrow::record_batch::RecordBatch> = stmt
        .query_arrow([])?
        .collect();

    // 3. 包装为 PyRecordBatchReader（零拷贝 via Arrow PyCapsule）
    //    Python 侧得到 pyarrow.lib.RecordBatchReader
    Ok(PyRecordBatchReader::from_iter(batches))
}
```

**Python 侧使用**：

```python
import pyarrow as pa

# Rust 返回的 PyRecordBatchReader 可直接转为各种 DataFrame 格式
reader = client.query_metric_series(run_ids=[...], metrics=["loss"])
table = pa.Table.from_batches(reader)        # pyarrow Table
df_polars = pl.from_arrow(table)             # polars DataFrame（零拷贝）
df_pandas = table.to_pandas()                # pandas DataFrame
```

**`pyo3-arrow` 提供的类型映射**：

| Rust struct | Python 类型 |
|---|---|
| `PyRecordBatchReader` | `pyarrow.RecordBatchReader` |
| `PyRecordBatch` | `pyarrow.RecordBatch` |
| `PyTable` | `pyarrow.Table` |
| `PyArray` | `pyarrow.Array` |
| `PyChunkedArray` | `pyarrow.ChunkedArray` |
| `PySchema` | `pyarrow.Schema` |

> 要求 Python 侧 `pyarrow>=14`（或 `>=15` 用于 `PyRecordBatchReader`）。也支持 `arro3` 作为轻量替代。

**duckdb-rs 的 Arrow API**：

```rust
// 方案 A: query_arrow — 物化所有 batch（适合小结果集）
let batches: Vec<RecordBatch> = stmt.query_arrow([])?.collect();

// 方案 B: stream_arrow — 懒加载流式迭代（适合大结果集，低内存）
let schema = stmt.schema();              // 先获取 schema
let stream = stmt.stream_arrow([], schema)?;  // 流式读取
for batch in stream { ... }
```

---

## 6. Rust Crate/模块结构

### 6.1 推荐结构：单 crate + 清晰模块划分

对于 v1 阶段，单 crate 是最简洁的选择。multi-crate workspace 增加编译复杂度和 crate 间的循环依赖风险，在接口尚未稳定时得不偿失。后期可拆分。

```
src/
├── lib.rs                    # crate root: 重新导出公共类型, pyo3 入口
│
├── model/                    # 逻辑数据模型（纯数据类型，无 I/O 依赖）
│   ├── mod.rs
│   ├── types.rs              # WorkspaceId, ProjectId, RunId, MetricName (newtype)
│   ├── run.rs                # Run, RunStatus, RunFilter, RunSort
│   ├── metric.rs             # MetricDefinition, MetricPoint, ValueType
│   ├── event.rs              # RunEvent, RunEventType
│   ├── summary.rs            # RunSummary, RunSummaryRow, SummaryFilter
│   ├── config.rs             # Config
│   ├── tag.rs                # Tag
│   └── artifact.rs           # Artifact
│
├── catalog/                  # CatalogLayer trait + Native 实现
│   ├── mod.rs
│   ├── trait.rs              # CatalogLayer trait 定义
│   ├── ducklake_impl.rs      # DuckLakeSqliteCatalog 实现
│   ├── cloud_impl.rs         # PostgresCatalog skeleton (future)
│   └── types.rs              # catalog 层特有类型 (FileInfo, SnapshotInfo, etc.)
│
├── storage/                  # StorageLayer trait + 实现
│   ├── mod.rs
│   ├── trait.rs              # StorageLayer trait 定义
│   ├── local.rs              # LocalStorage (读本地 Parquet)
│   ├── s3.rs                 # S3ObjectStorage (future)
│   └── ducklake_bridge.rs    # DuckLakeStorage (委托给 DuckLake)
│
├── compute/                  # ComputeLayer trait + 实现
│   ├── mod.rs
│   ├── trait.rs              # ComputeLayer trait 定义
│   ├── duckdb_impl.rs        # DuckDBCompute 实现
│   ├── cloud_impl.rs         # ClickHouseCompute skeleton (future)
│   └── query_interface.rs    # QueryInterface trait + DuckDBQueryInterface impl
│
├── engine/                   # 编排层：持有三层引用，实现写入/查询路径
│   ├── mod.rs
│   ├── client.rs             # PulseOnClient: 初始化、持有 catalog + compute
│   ├── write.rs              # 写入路径：log_metric, log_metrics, log_event, finish_run
│   ├── flush.rs              # flush 编排：run-end flush, compaction trigger
│   └── error.rs              # 统一错误类型
│
└── sdk/                      # PyO3 绑定层
    ├── mod.rs
    ├── client.rs             # #[pyclass] Python Client
    ├── run.rs                # #[pyclass] Python Run
    ├── config.rs             # 配置类（Python 侧 dict → Rust Config struct）
    ├── query.rs              # 查询结果返回 Python DataFrame/Arrow/dict
    └── error.rs              # Rust error → PyErr 转换
```

### 6.2 关键架构决策

1. **`model/` 零依赖**：纯数据结构，不依赖 catalog/storage/compute，任何层都可以引用。使用 newtype pattern（`RunId(String)`）避免字符串误用。

2. **`engine/` 是唯一编排点**：`engine::Client` 持有 `Arc<dyn CatalogLayer + StorageLayer + QueryInterface>` 并实现 `write.rs` 和 `flush.rs` 中的写入编排逻辑。其他模块不直接互相调用。

3. **`query_interface` 在 `compute/` 内**：因为查询接口的实现（SQL 翻译）与计算引擎强耦合。Native 版本的 `DuckDBQueryInterface` 持有 DuckDB connection，Cloud 版本的 `ClickHouseQueryInterface` 持有 HTTP client。

4. **Cloud 实现的 skeleton 与 Native 实现并存**：在 `catalog/cloud_impl.rs` 等文件中放置 trait 的空实现或 `unimplemented!()` skeleton，明确接口约定，方便后续填充。

5. **`pyo3` 依赖隔离在 `sdk/`**：核心模块（`model/`, `catalog/`, `storage/`, `compute/`, `engine/`）不依赖 `pyo3`。只有 `sdk/` 和 `lib.rs` 中的 `#[pymodule]` 入口依赖 `pyo3`。

### 6.3 Cargo.toml 依赖规划

```toml
[package]
name = "pulseon"
version = "0.1.0"
edition = "2024"

[lib]
name = "pulseon"
crate-type = ["cdylib"]       # 用于 Python 导入

[dependencies]
# Python binding
pyo3 = { version = "0.28", features = ["extension-module", "experimental-inspect"] }

# Arrow ↔ Python 零拷贝互操作（Arrow PyCapsule Interface）
pyo3-arrow = "0.17"

# DuckDB (embedded) — 版本号格式 1.MAJOR_MINOR_PATCH.x 对应 DuckDB v1.5.4
# "bundled": 静态编译 DuckDB C++ 库（maturin wheel 分发必需，避免系统依赖）
duckdb = { version = "~1.10504.0", features = ["bundled"] }

# Arrow（duckdb-rs 重新导出 arrow crate，但 pyo3-arrow 需要显式依赖）
arrow = "58"

# Async runtime
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Error handling
thiserror = "2"
anyhow = "1"

# Serde (for config, summary JSON)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Future: Cloud 版本（optional features）
# object_store = { version = "0.13", features = ["aws", "fs"], optional = true }
# sqlx = { version = "0.8", features = ["postgres"], optional = true }
# clickhouse-rs = { version = "...", optional = true }

[features]
default = ["native"]
native = []
# cloud = ["dep:object_store", "dep:sqlx", "dep:clickhouse-rs"]

[profile.release]
strip = true
```

**关键说明**：

- **`duckdb` 版本号**：duckdb-rs 从 DuckDB v1.5.0 起采用 `1.MAJOR_MINOR_PATCH.x` 编码格式。`~1.10504.0` 锁定 DuckDB v1.5.4。DuckLake 需要 DuckDB v1.1+，v1.5.4 完全支持。
- **`bundled` feature**：从源码编译 DuckDB C++ 库，静态链接到 Rust 二进制。maturin wheel 分发必需——用户 `pip install` 即可用，无需系统安装 libduckdb。代价：首次编译增加约 1-2 分钟，wheel 体积增加约 15-20MB。
- **`httpfs` 不是 cargo feature**：S3/http 文件支持是 DuckDB 的 runtime 扩展，通过 SQL `INSTALL httpfs; LOAD httpfs;` 加载，不需要在 Cargo.toml 中声明。
- **`ducklake` 同理**：DuckLake 扩展也通过 SQL `INSTALL ducklake; LOAD ducklake;` 加载（DuckDB v1.5+ 支持首次 ATTACH 时自动加载）。
- **`pyo3-arrow`**：基于 Arrow PyCapsule Interface 实现零拷贝。要求 Python 侧 `pyarrow>=14`。

---

## 7. Rust vs Python 职责边界

### 7.1 Rust Core 拥有

| 职责 | 说明 |
|------|------|
| **Layer 生命周期** | DuckDB 连接创建/销毁、DuckLake ATTACH/DETACH、CHECKPOINT |
| **写入路径** | 接收 metric_point → 验证 metric_definition → 拼接 `INSERT INTO dl.metric_points` → 执行 |
| **Flush 编排** | run.finish() → 计算 summary → `CALL ducklake_flush_inlined_data(...)` → 可选 compact |
| **查询执行** | SQL 生成、DuckDB 查询执行、Arrow 结果组装 |
| **Catalog 管理** | CREATE TABLE、注册 metric_definition、元数据 CRUD |
| **Storage 抽象** | Parquet 文件读写（导出/迁移场景） |
| **并发/锁** | 单 writer v1 无并发需求；未来多线程用 `Mutex<Connection>` 或连接池 |
| **类型安全** | newtype ID、ValueType 枚举、Result 错误类型 |
| **数据验证** | step 单调性检查、value_type 一致性检查 |

### 7.2 Python SDK 拥有

| 职责 | 说明 |
|------|------|
| **用户 API** | 友好的 `init()` / `create_run()` / `log_metric()` 接口 |
| **训练框架集成** | PyTorch Lightning callback、HuggingFace callback、Keras callback |
| **DataFrame/Numpy 互转** | 返回 `pandas.DataFrame`、`polars.DataFrame`、`numpy.ndarray` |
| **Config 对象** | dict → config 的便利方法 |
| **Type hints** | 完整的 `.pyi` stub 或 inline annotations |
| **Auto-logging** | 自动捕获系统 metric（CPU/GPU util、内存、磁盘）、自动 log model graph |
| **上下文管理器** | `with pulseon.Run(...) as run:` 自动 finish |
| **Logging 桥接** | Python `warnings` / `logging` → 可选的事件写入 |

### 7.3 PyO3 暴露面（`src/sdk/`）

```rust
// lib.rs 中的 #[pymodule]
#[pymodule]
fn pulseon(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<sdk::client::Client>()?;
    m.add_class::<sdk::run::Run>()?;
    m.add_class::<sdk::query::QueryResult>()?;
    m.add_function(wrap_pyfunction!(sdk::client::init, m)?)?;
    Ok(())
}
```

**暴露给 Python 的类型**：
- `Client` — 初始化、create_run、list_runs、list_projects、query_*
- `Run` — log_metric、log_metrics、log_event、finish、update_summary
- `QueryResult` — to_pandas()、to_polars()、to_arrow()、to_dicts()

**不暴露给 Python 的**（Rust-internal）：
- `CatalogLayer` / `StorageLayer` / `ComputeLayer` trait 本身
- DuckDB connection 管理
- DuckLake 内部操作（ATTACH、CHECKPOINT）
- SQL 生成
- Flush/compact 编排
- Arrow RecordBatch 中间处理

---

## 8. Python SDK API 表面

### 8.1 核心 API sketch

```python
# pulseon/__init__.py (Python 侧薄封装)
import pulseon._core  # PyO3 编译产物

class Client:
    """PulseOn 客户端——Python 侧用户 API"""

    def __init__(self, rust_client: pulseon._core.Client):
        self._rust = rust_client

    def create_run(
        self,
        project: str,
        name: str | None = None,
        config: dict | None = None,
        tags: list[str] | None = None,
    ) -> "Run":
        """创建新的训练 run"""
        ...

    def list_runs(
        self,
        project: str | None = None,
        # filters, sort, limit...
    ) -> list[dict]:
        """列出 runs"""
        ...

    def query_metric_series(
        self,
        run_ids: list[str],
        metrics: list[str],
        step_range: tuple[int, int] | None = None,
    ) -> "pl.DataFrame":
        """查询指标时序数据，返回 polars DataFrame"""
        ...


class Run:
    """单个训练 run 的句柄"""

    def __init__(self, rust_run: pulseon._core.Run, client: Client):
        self._rust = rust_run
        self._client = client

    @property
    def id(self) -> str: ...

    @property
    def name(self) -> str: ...

    def log_metric(
        self,
        name: str,
        step: int,
        value: float | int | str | bool,
        phase: str | None = None,
        timestamp: datetime | None = None,
    ) -> None:
        """记录单个 metric 值"""
        ...

    def log_metrics(
        self,
        metrics: dict[str, float | int],
        step: int,
        phase: str | None = None,
    ) -> None:
        """批量记录多个 metric 值（更高效）"""
        ...

    def log_event(
        self,
        event_type: str,  # 'checkpoint', 'alert', 'user_note'
        message: str | None = None,
        metadata: dict | None = None,
    ) -> None: ...

    def finish(self) -> None:
        """结束 run，触发 flush"""
        ...

    def __enter__(self) -> "Run": ...

    def __exit__(self, *args) -> None:
        self.finish()


def init(
    path: str,                 # 本地路径 或 "s3://bucket/prefix"
    workspace: str = "default",
    # s3 配置（仅当 path 为 s3:// 时需要）
    s3_endpoint: str | None = None,
    s3_access_key: str | None = None,
    s3_secret_key: str | None = None,
    s3_region: str | None = None,
) -> Client:
    """初始化 PulseOn client——Native 模式的唯一入口"""
    ...
```

### 8.2 典型使用流程

```python
import pulseon

# 本地模式
client = pulseon.init("./my_training_runs")
run = client.create_run(project="image-classifier", name="resnet50-bs32")

# 训练循环
for step, batch in enumerate(dataloader):
    loss = model(batch)
    run.log_metric("train/loss", step=step, value=loss)
    if step % 100 == 0:
        acc = evaluate(model)
        run.log_metrics({"val/acc": acc, "val/loss": val_loss}, step=step)

run.finish()

# 查询
loss_series = client.query_metric_series(
    run_ids=[run.id],
    metrics=["train/loss"],
)
# loss_series 是 polars DataFrame:
#   run_id | metric_name | step | timestamp | value_f64
```

```python
# S3 模式（自建 MinIO 或其他 S3 兼容存储）
client = pulseon.init(
    "s3://my-bucket/pulseon-data",
    s3_endpoint="http://minio.local:9000",
    s3_access_key="...",
    s3_secret_key="...",
)

# 上下文管理器模式
with client.create_run(project="llm-finetune", name="llama3-lora") as run:
    for step in range(1000):
        run.log_metrics({"loss": ..., "lr": ...}, step=step)
# 自动 finish
```

### 8.3 Rust 侧暴露面

```rust
// src/sdk/client.rs
#[pyclass]
pub struct Client {
    inner: Arc<engine::PulseOnClient>,
}

#[pymethods]
impl Client {
    #[new]
    fn new(config: PyConfig) -> PyResult<Self> { ... }

    fn create_run(&self, project: &str, name: Option<&str>,
                  config: Option<PyObject>, tags: Option<Vec<String>>)
                  -> PyResult<Run> { ... }

    fn list_runs(&self, project: Option<&str>) -> PyResult<Vec<PyObject>> { ... }

    fn query_metric_series(&self, run_ids: Vec<String>, metrics: Vec<String>,
                           step_start: Option<i64>, step_end: Option<i64>)
                           -> PyResult<PyRecordBatchReader> { ... }
}

// src/sdk/run.rs
#[pyclass]
pub struct Run {
    inner: Arc<engine::RunHandle>,
}

#[pymethods]
impl Run {
    fn log_metric(&self, name: &str, step: i64, value: Bound<'_, PyAny>,
                  phase: Option<&str>) -> PyResult<()> { ... }

    fn log_metrics(&self, metrics: HashMap<String, Bound<'_, PyAny>>,
                   step: i64, phase: Option<&str>) -> PyResult<()> { ... }

    fn finish(&self) -> PyResult<()> { ... }
}
```

---

## 9. 配置与部署模式

### 9.1 配置结构

```rust
// src/model/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseOnConfig {
    /// 部署模式
    pub mode: DeploymentMode,

    /// 工作空间名称
    pub workspace: String,

    /// 存储路径（本地路径 或 s3:// URI）
    pub data_path: String,

    /// Catalog 配置
    pub catalog: CatalogConfig,

    /// DuckDB 配置
    pub duckdb: DuckDBConfig,

    /// Flush 配置
    pub flush: FlushConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeploymentMode {
    /// 本地模式：catalog=SQLite, storage=本地或S3, compute=DuckDB
    Local,

    // Future:
    // /// 云端模式：catalog=PG, storage=S3, compute=ClickHouse
    // Cloud { api_key: String, endpoint: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    /// SQLite catalog 文件路径（相对 data_path 或绝对路径）
    pub sqlite_path: Option<String>,  // None → 默认 data_path/catalog.sqlite

    /// DuckLake 内联数据行数阈值
    pub data_inlining_row_limit: Option<u32>,  // None → 默认 500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuckDBConfig {
    /// DuckDB 内存限制
    pub memory_limit: Option<String>,  // e.g. "4GB"

    /// 线程数（默认 1，训练场景避免与训练进程争抢 CPU）
    pub threads: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlushConfig {
    /// 自动 flush 时间间隔（秒），None = 禁用定期 flush
    pub auto_flush_interval_secs: Option<u64>,

    /// 自动 flush 行数阈值
    pub auto_flush_row_threshold: Option<u64>,

    /// Run 结束时是否 compact
    pub compact_on_finish: bool,
}
```

### 9.2 Python 配置映射

```python
import pulseon

# 最简配置：本地模式，所有默认值
client = pulseon.init("./runs")

# 详细配置
client = pulseon.init(
    path="./runs",
    workspace="research-lab",
    config={
        "catalog": {
            "data_inlining_row_limit": 1000,
        },
        "duckdb": {
            "memory_limit": "2GB",
            "threads": 2,
        },
        "flush": {
            "auto_flush_interval_secs": 60,
            "compact_on_finish": True,
        },
    },
)

# S3 模式
client = pulseon.init(
    path="s3://my-bucket/pulseon",
    s3_endpoint="https://s3.amazonaws.com",  # AWS S3
    # 或自建 MinIO:
    # s3_endpoint="http://192.168.1.100:9000",
    s3_access_key="AKIA...",
    s3_secret_key="...",
    s3_region="us-east-1",
)
```

### 9.3 未来 Cloud 模式配置（sketch）

```python
# 未来 Cloud 版本
client = pulseon.init(
    mode="cloud",
    api_key="pulseon_sk_...",
    endpoint="https://api.pulseon.cloud",
    # 可选：自带 bucket
    storage={
        "type": "s3",
        "bucket": "my-company-bucket",
        "region": "us-east-1",
        "access_key": "...",
    },
)
```

### 9.4 初始化流程

```
pulseon.init(path, workspace, config)
  │
  ├─ 1. 解析 DeploymentMode
  │      path 以 "s3://" 开头？
  │      ├─ YES: 标记需要 httpfs + S3 配置
  │      └─ NO:  本地文件模式
  │
  ├─ 2. 确定 catalog 文件位置
  │      catalog.sqlite 在 data_path 目录下
  │
  ├─ 3. 创建 DuckDB connection (in-memory)
  │      duckdb::Connection::open_in_memory()
  │
  ├─ 4. 加载扩展（通过 SQL，非 cargo feature）
  │      INSTALL ducklake; LOAD ducklake;
  │      INSTALL lttb; LOAD lttb;       ← 降采样扩展（agent get_metric_digest 热路径）
  │      (如果需要 S3) INSTALL httpfs; LOAD httpfs;
  │
  ├─ 5. 配置 S3 (如果需要)
  │      SET s3_endpoint = '...'; SET s3_access_key_id = '...'; ...
  │
  ├─ 6. ATTACH DuckLake catalog
  │      ATTACH '<catalog_path>' AS dl (TYPE ducklake, CATALOG 'sqlite',
  │                                      DATA_PATH '<data_path>',
  │                                      DATA_INLINING_ROW_LIMIT 500)
  │
  ├─ 7. 初始化业务表 (如果不存在)
  │      CREATE TABLE IF NOT EXISTS dl.runs (...)
  │      CREATE TABLE IF NOT EXISTS dl.metric_definitions (...)
  │      CREATE TABLE IF NOT EXISTS dl.metric_points (...)
  │      ALTER TABLE dl.metric_points SET PARTITIONED BY (...)
  │      ...
  │
  ├─ 8. 创建 Rust 层对象
  │      let catalog = DuckLakeSqliteCatalog::new(conn.clone());
  │      let compute = DuckDBCompute::new(conn.clone());
  │      let client = PulseOnClient::new(catalog, compute);
  │
  └─ 9. 包装为 Python Client 返回
```

---

## 10. 关键风险、权衡与开放问题

### 10.1 DuckLake 作为 catalog 机制的风险

| 风险 | 严重度 | 缓解措施 |
|------|--------|---------|
| DuckLake 被 DuckDB Labs 弃用或方向改变 | 中 | DuckLake 是 DuckDB 的一等扩展（非第三方），且 catalog 逻辑通过 `CatalogLayer` trait 封装，可迁移到自建 catalog |
| DuckLake 版本升级导致 catalog 不兼容 | 中 | DuckLake snapshot 协议稳定（1.0），Parquet 文件是标准格式；最坏情况：迁移工具通过 Parquet 重新导入 |
| DuckLake 不支持分区级 flush | 低 | Run-centric 场景 flush 在 run 结束时执行，全表 flush 足够；未来大量并行 run 可能需要额外优化 |
| DuckLake 元数据膨胀（快照太多） | 低 | v1 单 writer 快照数量可控；可定期 CHECKPOINT 并清理旧快照 |

### 10.2 DuckDB 扩展加载可行性（已验证 ✅）

duckdb-rs 完全支持加载 DuckLake 和 httpfs 扩展。验证结论：

- **加载方式**：通过 `conn.execute_batch("INSTALL ducklake; LOAD ducklake;")` 执行 SQL，或使用 `Connection::load_extension()` API。DuckDB v1.5+ 支持首次 ATTACH 时自动加载 DuckLake。
- **ATTACH 语法**：`conn.execute_batch("ATTACH '...' AS dl (TYPE ducklake, ...)")` 完全支持。
- **版本**：duckdb-rs 最新版 `~1.10504.0` 捆绑 DuckDB v1.5.4，完全支持 DuckLake 1.0。
- **生产实践**：windmill（~30k stars）和 supabase/etl 均在生产中使用 duckdb-rs + DuckLake，模式为解析 SQL 中的 ATTACH 语句、解析凭证、通过 duckdb-rs 执行。
- **无 DuckLake 专属 Rust 绑定**：DuckLake 是 DuckDB 扩展，所有访问通过 SQL 接口，这是设计路径而非临时方案。

### 10.3 DuckLake + DuckDB 运行时耦合 vs Cloud 可替换性

**这个耦合不影响 Cloud 版本**。理由：
- Native 版本 = DuckLake(SQLite) + DuckDB（同一进程）
- Cloud 版本 = PostgreSQL + S3 + ClickHouse（完全不同的进程和技术栈）
- Cloud 版本不需要任何 DuckDB/DuckLake 依赖
- Rust trait 层面，`PostgresCatalog` 和 `ClickHouseCompute` 是完全独立的实现

**耦合只在 Native 版本内部**，是两个 trait 实现共享一个 DuckDB Connection。这可以通过 `Arc<duckdb::Connection>` 安全共享，不需要特别的解耦。

### 10.4 SQLite 访问约束

DuckLake 通过 DuckDB 内部的 SQLite reader 管理 catalog 文件。**禁止使用 `rusqlite` 直接访问同一个 SQLite 文件**，否则会导致：
- 文件级锁冲突（SQLite WAL 模式允许多读单写）
- 页面缓存不一致（DuckDB 和 rusqlite 各有独立 page cache）
- DuckLake schema 变更时崩溃

**规则**：所有 SQLite 访问必须通过 DuckDB/DuckLake 的 SQL 接口。如果需要独立的配置数据库，使用单独的 SQLite 文件（此时可用 `rusqlite`）。

### 10.5 单 Writer 假设

v1 设计所有路径假设**单 writer**（一个进程同时只有一个活跃的 run 在写入，或至多一个 run 一个线程）。如果用户同时运行多个训练脚本写入同一 catalog：

- **SQLite catalog backend**：SQLite 的并发写入有已知限制（WAL mode 允许多读单写）
- **DuckLake 快照**：并发写入可能导致快照冲突
- **内联数据**：不同 run 的内联数据在同一 DuckLake 中，但 DuckDB 是单线程 SQL 执行，无资源竞争

**v1 缓解**：
- 文档化"一个 data_path 一个 writer"
- 如果用户需要多 run 并行，建议启动多个 Python 进程，每个指向独立的 data_path
- 未来 v2 通过 PostgreSQL catalog 支持多 writer

### 10.6 小文件管理

即使保持内联开启，flush 后可能产生多个小 Parquet 文件（尤其是不同 metric 的 flush 时间不同）。缓解：

1. **内联阈值 500-1000** 意味着至少积累几百行才 flush，flushed 文件不会太小
2. **Run 结束时 compact**：`ducklake_merge_adjacent_files` 合并同分区小文件
3. **分区设计**：按 `(project_id, run_id, metric_name)` 分区意味着同一 metric 的数据在同一文件/目录，查询效率高
4. **未来考虑**：background compaction（独立线程定期扫描和合并）

### 10.7 内存管理

训练进程本身消耗大量内存（GPU 显存 + CPU 内存），PulseOn 的 DuckDB 不应成为额外负担：

- DuckDB `memory_limit` 默认限制（建议 512MB-1GB）
- DuckDB 线程数默认 1（`SET threads = 1`），避免与训练框架争抢 CPU
- 内联数据积累在 DuckDB 内存中，大量 step 不 flush 可能撑爆。定期自动 flush（`auto_flush_interval_secs`）可缓解
- 对于超长训练（100万 step+），考虑每个 metric 独立 flush 或 streaming flush

### 10.8 平台兼容性注意事项

| 问题 | 平台 | 缓解措施 |
|------|------|---------|
| `pyarrow` import 顺序导致 DuckDB 初始化崩溃 | Windows | 在 Python 侧先 `import pulseon` 再 `import pyarrow`；或确保 duckdb-rs ≥1.4.0 + pyarrow ≥20.0.0 |
| ICU 扩展未包含在 `bundled` feature 中 | 全平台 | 需要日期/时间操作时在 runtime `INSTALL icu; LOAD icu;`（需网络）；或使用 `bundled-cmake`（git checkout）静态编译 ICU |
| DuckLake 扩展预编译二进制可用性 | macOS ARM64 / Linux x86_64 | DuckDB v1.5+ 的扩展仓库覆盖主流平台；离线场景需预下载 `.duckdb_extension` 文件 |
| Maturin wheel 体积（DuckDB bundled 约 +15-20MB） | 全平台 | release profile 使用 `strip = true`；可接受的成本 |

### 10.9 关于 DuckLake 与 reference doc 矛盾的最终说明

Reference doc 说"不直接依赖 DuckLake 作为底层协议"。这个判断的上下文是：**定义跨所有部署形态的统一产品协议**。在这个上下文中，依赖 DuckLake 作为协议会：
- 把 ClickHouse 导入路径复杂化（需要额外处理 DuckLake 元数据）
- 增加通用性负担（DuckLake 处理多 writer、删除向量等通用 lakehouse 问题）

但是 Native 版本选择用 DuckLake **不是**用它作为产品协议，而是用它作为 Native 版本的**实现工具**。关键区别：
- **产品协议** = 逻辑模型 + Parquet 格式 + catalog 概念 → 这些是 PulseOn 定义的，不依赖 DuckLake
- **实现工具** = DuckLake 帮我们管理 SQLite 中的元数据、管理 Parquet 文件、管理内联/flush → 这是实现细节，被 trait 封装

Cloud 版本不需要 DuckLake。Parquet 文件是标准的。`CatalogLayer` trait 封装了 DuckLake 的存在。这就是调和方案。

---

## 附录：快速对照表

| 决策点 | Reference Doc | 本设计 | 理由 |
|--------|--------------|--------|------|
| Catalog 后端 | 自建 SQL catalog（SQLite/PG） | DuckLake + SQLite（Native），直接 PG（Cloud） | DuckLake 提供内联/flush/分区等开箱能力，避免在 v1 自建 staging 层 |
| Staging 机制 | 自建 staging 表或 WAL | DuckLake Data Inlining | 已验证可靠，零额外代码 |
| DuckLake 依赖 | 不直接依赖 | Native 版本依赖，通过 trait 封装 | 调和方案：实现工具非产品协议 |
| 目录组织 | hive: project/run/metric | 同，DuckLake auto-managed | 一致 |
| metric_points | 长表，typed value columns | 同 | 一致 |
| 统一查询接口 | Local/Cloud/SelfHosted adapter | QueryInterface trait | 一致，adapter 模式内化到 trait |
| v1 范围 | 单 writer, append-only, no DELETE/UPDATE | 同 | 一致 |
| 交换格式 | Parquet + zstd | 同（DuckLake flush 默认 zstd） | 一致 |

---

## 附录：技术选型验证依据

| 组件 | 选型 | 验证状态 | 来源 |
|------|------|---------|------|
| duckdb-rs | `~1.10504.0` (DuckDB v1.5.4) | ✅ 生产成熟，918 stars，DuckDB Labs 工程师维护 | duckdb-rs GitHub |
| DuckLake 扩展加载 | SQL `INSTALL`/`LOAD`/`ATTACH` | ✅ windmill + supabase/etl 生产实践 | GitHub 源码验证 |
| duckdb-lttb 扩展 | SQL `INSTALL lttb; LOAD lttb;` | ✅ 自研扩展，benchmark 超越 ClickHouse，99 条测试通过 | duckdb-lttb 项目 |
| PyO3 Arrow 互操作 | `pyo3-arrow` v0.17 (Arrow PyCapsule) | ✅ 零拷贝，支持 pyarrow/polars/arro3 | docs.rs/pyo3-arrow |
| S3 对象存储 (Cloud) | `object_store` crate v0.13 | ✅ Apache Arrow 生态，5.2M 月下载 | docs.rs/object_store |
| SQLite 访问 | 全部走 DuckDB/DuckLake | ✅ 避免 rusqlite 并发锁冲突 | DuckLake 验证存档 |

---

**文档版本**: v2.0（Part I）
**参考文档**: `docs/reference/training-metrics-hosting-boundaries.md` · `docs/reference/training-metrics-storage-architecture.md` · `docs/reference/ducklake-archive.md`

---

# Part II：AI Native 扩展

> 版本：v2.0
> 定位：在 Part I 基础架构之上，增加 AI Agent 作为一等用户的设计层。非重写，是对三层架构、数据模型、查询接口和 SDK 的语义扩展。
> 核心目标：借助 PulseOn，用户能真正实现 auto research——LLM-based agent 自主回顾实验、提出假设、启动训练、监控指标、分析结果、生成洞察、迭代推进。

---

## 11. AI Native 设计哲学

### 11.1 为什么 "AI Native" 是一等架构关注，不是功能补丁

训练指标追踪产品的核心价值是"帮助用户从实验中学习并做出更好的下一步决策"。传统产品把人类作为唯一决策者。AI Native 意味着：**把 LLM-based agent 提升为与人类对等的系统用户**。

这不是在现有 SDK 上加几个便利函数。它要求：

1. **数据模型必须承载 agent 的认知状态**——agent 不仅记录"loss=0.3"，还需要记录"为什么启动这个实验、我预期什么、我观察到了什么、我决定下一步做什么"。
2. **查询接口必须有语义层**——agent 不需要 100k 个 raw data point，它需要"这个实验比上一个好吗？哪里出现了异常？我应该关注哪个 metric？"
3. **输出格式必须适配 LLM 上下文窗口**——Arrow RecordBatch 对人类数据科学家友好，对 LLM 是噪音。
4. **必须提供 tool-calling schema 和 MCP server**——让 LLM 能通过 function calling 或 MCP 协议直接操作 PulseOn，像调用天气 API 一样自然。

### 11.2 三种用户画像

| 画像 | 典型操作 | 数据需求 | 输出偏好 |
|------|---------|---------|---------|
| **Developer** | `client.init()`, `create_run()`, `log_metric()` | 原始 metric_points（Arrow） | DataFrame/Arrow（用于 Python 分析） |
| **Trainer** | 通过 PyTorch Lightning / Keras callback 自动记录 | 折线图数据、run 对比 | 可视化（由前端消费） |
| **AI Agent** | `find_best_runs()`, `summarize_run()`, `compare_runs()`, `get_metric_digest()` | 降采样序列、统计摘要、异常标记、语义总结 | JSON / Markdown / 紧凑序列（适配 8K-128K token 上下文窗口） |

**关键差异**：Developer 和 Trainer 走 `QueryInterface` trait（返回 Arrow），AI Agent 走 `AgentToolInterface` trait（返回语义化结构）。

### 11.3 "Auto Research" 的具体含义

Auto research 不是一个黑盒"AI 自动做实验"，而是一个**人机协作的迭代循环**，架构支持的每个步骤：

```
┌─────────────────────────────────────────────────────────────┐
│                    Auto Research Loop                        │
│                                                             │
│  1. REVIEW ──── Agent 阅读 project 历史                      │
│       │         - list_runs, summarize_project,              │
│       │           get_insights, get_experiment_lineage       │
│       ▼                                                     │
│  2. HYPOTHESIZE  Agent 提出假设                               │
│       │         - create_hypothesis, recommend_next_experiment│
│       ▼                                                     │
│  3. EXECUTE ── Agent 启动训练 run（带 hypothesis 元数据）     │
│       │         - create_run(config, hypothesis_id)           │
│       ▼                                                     │
│  4. MONITOR ── Agent 实时监控指标                             │
│       │         - get_recent_metrics, detect_anomaly          │
│       ▼                                                     │
│  5. DECIDE ─── Agent 做出干预决策                             │
│       │         - log_agent_decision                         │
│       │         - finish_run if converged                    │
│       ▼                                                     │
│  6. ANALYZE ── Agent 分析结果                                 │
│       │         - compare_runs, get_metric_digest             │
│       ▼                                                     │
│  7. INSIGHT ── Agent 生成洞察 + 报告                         │
│       │         - add_insight, generate_report               │
│       ▼                                                     │
│  8. ITERATE ── 回到步骤 2，基于新洞察提出下一个假设          │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**行业验证**：此循环与已验证的 auto-research 模式一致——AIDE（[WecoAI/aideml](https://github.com/WecoAI/aideml)，4K+ stars）使用 tree search + LLM-guided branching 在 Kaggle 竞赛中击败 50% 参赛者；Karpathy 的 [AutoResearch](https://github.com/karpathy/AutoResearch) 使用线性迭代 + git-based tracking，8 小时内自主完成 50-100 个实验。PulseOn 的设计吸收两者的核心模式：AIDE 的实验谱系追踪 + AutoResearch 的快速迭代循环。

### 11.4 关键张力：LLM 上下文窗口 vs 原始时序数据

| 数据类型 | 典型大小 | LLM 能否处理 | 解决方案 |
|---------|---------|------------|---------|
| 单个 run 的 metric_points | 10K-100K 行 | ❌ 超出上下文窗口 + token 成本爆炸 | LTTB 降采样到 50-200 点 |
| 多个 run 的 metric_points | 100K-1M 行 | ❌ | `compare_runs()` 返回统计摘要，非原始数据 |
| project 下的 run 列表 | 10-1000 个 | ⚠️ 大量时需过滤 | `find_best_runs(top_k=5)`, 按 metric 排序 |
| run_summary + insights | 1-5KB 文本 | ✅ | JSON/Markdown 直接可消费 |
| 降采样 metric digest | 50-200 点 | ✅ | 紧凑 JSON 数组 |

**设计原则**：所有 `AgentToolInterface` 方法默认返回不超过 2000 token（~6000 字符）的紧凑数据。原始数据查询走 `QueryInterface`。

---

## 12. 数据模型扩展（Auto Research）

### 12.1 更新后的实体关系图

```
Workspace 1──N Project 1──N Hypothesis ──N Run
                                    │         │
                    ┌───────────────┼─────────┼────────────────────┐
                    │               │         │                    │
            MetricDefinition  metric_point  RunEvent        parent_run_id
            (1 metric/run)   (N points/run)                (自引用：实验树)
                    │               │                           │
            RunSummary    MetricDigest    AgentDecision    Insight    Report
            (1 per run)   (pre-computed)  (agent trail)  (per run)  (per project/run)
```

### 12.2 新增表 Schema

所有新增表均为 **DuckLake catalog table**（无分区，行式存储），因为数据量小（每 project 总计 <10MB）、需要事务性 CRUD、SQLite/PostgreSQL 行式查询在此规模下高效。

#### `hypotheses`（catalog 表）

Agent 或人类在启动实验前记录的假设——auto research 的**意图锚点**。

```sql
CREATE TABLE hypotheses (
    id            VARCHAR PRIMARY KEY,        -- UUID
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    title         VARCHAR NOT NULL,           -- 简短标题："Higher LR reduces plateau"
    description   VARCHAR,                    -- 详细描述（markdown），LLM 可阅读
    independent_variable VARCHAR,             -- 自变量："learning_rate"
    expected_outcome     VARCHAR,             -- 预期结果："Loss converges < 0.1 within 5000 steps"
    source_type   VARCHAR NOT NULL,           -- 'human', 'agent'
    source_insight_id VARCHAR,               -- 如果基于某个 insight 提出
    agent_model   VARCHAR,                    -- 'claude-4', 'gpt-4o'
    status        VARCHAR NOT NULL DEFAULT 'proposed',  -- proposed, testing, confirmed, rejected
    created_at    TIMESTAMP NOT NULL DEFAULT now(),
    updated_at    TIMESTAMP NOT NULL DEFAULT now()
);
```

#### `experiment_lineage`（catalog 表）

记录 run 之间的派生关系——"run B 是 run A 的变体，因为 insight X"。查询模式：递归 CTE 向上/向下遍历实验树。

```sql
CREATE TABLE experiment_lineage (
    id            VARCHAR PRIMARY KEY,
    run_id        VARCHAR NOT NULL,           -- 子 run
    parent_run_id VARCHAR NOT NULL,           -- 父 run
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    reason        VARCHAR NOT NULL,           -- e.g. "tune learning_rate from 0.001 to 0.01"
    hypothesis_id VARCHAR,                    -- 关联的假设
    diff_summary  VARCHAR,                    -- config 差异摘要（JSON 或简短文本）
    created_at    TIMESTAMP NOT NULL DEFAULT now(),
    UNIQUE(run_id, parent_run_id)
);
```

#### `agent_decisions`（catalog 表）

记录 agent 的认知轨迹——每个决策（停止训练、调整 LR、启动新 run）都是结构化的推理日志。与 `run_event` 不同：`run_event` 记录训练过程中发生的事，`agent_decision` 记录 agent 的推理和决策依据。

```sql
CREATE TABLE agent_decisions (
    id            VARCHAR PRIMARY KEY,
    run_id        VARCHAR NOT NULL,
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    decision_type VARCHAR NOT NULL,           -- 'stop_training'|'adjust_hyperparam'|'launch_new_run'|'mark_anomaly'|'generate_insight'
    reasoning     VARCHAR NOT NULL,           -- 推理过程（markdown），LLM 可读
    data_snapshot VARCHAR,                    -- 决策依据的快照数据（JSON）
    action_taken  VARCHAR,                    -- 实际执行了什么操作
    result        VARCHAR,                    -- 操作结果
    agent_model   VARCHAR,                    -- 'claude-4', 'gpt-4o'
    agent_version VARCHAR,
    created_at    TIMESTAMP NOT NULL DEFAULT now()
);
```

#### `insights`（catalog 表）

Agent 或人类生成的洞察——对 run 或 project 的分析结论。

```sql
CREATE TABLE insights (
    id            VARCHAR PRIMARY KEY,
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    run_id        VARCHAR,                    -- NULL = project-level insight
    type          VARCHAR NOT NULL,           -- 'finding'|'recommendation'|'anomaly'|'conclusion'
    title         VARCHAR NOT NULL,
    content       VARCHAR NOT NULL,           -- 详细内容（markdown）
    evidence      VARCHAR,                    -- 支持洞察的数据摘要（JSON）
    severity      VARCHAR,                    -- 'critical'|'warning'|'info'
    source_type   VARCHAR NOT NULL,           -- 'human'|'agent'
    source_decision_id VARCHAR,              -- 从哪个 agent decision 产生
    status        VARCHAR NOT NULL DEFAULT 'active',  -- active, archived, resolved
    created_at    TIMESTAMP NOT NULL DEFAULT now(),
    updated_at    TIMESTAMP NOT NULL DEFAULT now()
);
```

#### `reports`（catalog 表）

Agent 自动生成的 markdown 分析报告。

```sql
CREATE TABLE reports (
    id            VARCHAR PRIMARY KEY,
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    run_ids       VARCHAR,                    -- JSON array of run IDs
    title         VARCHAR NOT NULL,
    content       VARCHAR NOT NULL,           -- markdown 全文
    template      VARCHAR,                    -- 使用的报告模板名称
    generated_by  VARCHAR NOT NULL,           -- 'agent', agent model name
    related_insight_ids VARCHAR,             -- JSON array of insight IDs
    created_at    TIMESTAMP NOT NULL DEFAULT now()
);
```

#### `metric_digests`（catalog 表——预计算降采样数据）

为每个 run 的每个 metric 预计算降采样序列，agent 查询时直接读取，避免 100k 点全量扫描。**按需计算 + 缓存**模式：`get_metric_digest()` 先查缓存，没有则从 `metric_points` 计算并写入。run finish 时预计算一批 digest。

```sql
CREATE TABLE metric_digests (
    id            VARCHAR PRIMARY KEY,
    run_id        VARCHAR NOT NULL,
    project_id    VARCHAR NOT NULL,
    workspace_id  VARCHAR NOT NULL,
    metric_name   VARCHAR NOT NULL,
    max_points    INTEGER NOT NULL,           -- 保留的最大点数（如 50, 100, 200）
    algorithm     VARCHAR NOT NULL,           -- 'lttb' | 'bucket_stats'
    data          VARCHAR NOT NULL,           -- JSON: [[step, value], ...] 或 bucket stats
    total_points  BIGINT,                     -- 原始数据的点数
    value_min     DOUBLE,
    value_max     DOUBLE,
    value_mean    DOUBLE,
    value_last    DOUBLE,
    trend         VARCHAR,                    -- 'improving'|'degrading'|'stable'|'volatile'
    computed_at   TIMESTAMP NOT NULL DEFAULT now(),
    UNIQUE(run_id, metric_name, max_points, algorithm)
);
```

#### `runs` 表扩展字段

```sql
ALTER TABLE dl.runs ADD COLUMN IF NOT EXISTS hypothesis_id VARCHAR;   -- 关联假设
ALTER TABLE dl.runs ADD COLUMN IF NOT EXISTS parent_run_id VARCHAR;    -- 父 run（实验树快速访问）
```

`parent_run_id` 是 `experiment_lineage` 的冗余字段，用于快速 `WHERE parent_run_id = ?` 查询。完整树遍历通过 `experiment_lineage` 递归 CTE。

### 12.3 表存储位置汇总

| 表 | 存储类型 | 分区 | 数据量估计 | 写入模式 |
|---|---|---|---|---|
| `hypotheses` | DuckLake catalog table | 无 | 每 project 10-100 行 | CRUD |
| `experiment_lineage` | DuckLake catalog table | 无 | 每 project 50-500 行 | append |
| `agent_decisions` | DuckLake catalog table | 无 | 每 run 5-50 行 | append |
| `insights` | DuckLake catalog table | 无 | 每 project 20-200 行 | CRUD |
| `reports` | DuckLake catalog table | 无 | 每 project 1-20 行 | append |
| `metric_digests` | DuckLake catalog table | 无 | 每 run × metric × digest 变体 | 计算 + upsert |

如果某个表未来数据量意外增长（如 `agent_decisions` 达到百万行），可通过 `ALTER TABLE ... SET PARTITIONED BY (project_id)` 转为 Parquet 分区表，无需应用层改动。

---

## 13. 语义查询层（AgentToolInterface trait）

### 13.1 Trait 定义

```rust
use serde::{Serialize, Deserialize};

/// AI Agent 专用的语义查询接口。
/// 区别于 QueryInterface（返回 Arrow RecordBatch 给人类/数据科学用途），
/// AgentToolInterface 返回 LLM 可直接消费的紧凑结构化数据。
///
/// 设计原则：
/// - 所有方法返回 JSON/Markdown/紧凑 struct（不是 Arrow）
/// - 默认输出 token 预算 ≤ 2000 tokens（约 6000 字符）
/// - 方法命名对标 LLM tool-calling 的直觉语义
#[async_trait::async_trait]
pub trait AgentToolInterface: Send + Sync {
    // ── Run 发现与排名 ───────────────────────────

    /// 查找 project 下某个 metric 表现最好的 top-k 个 run
    async fn find_best_runs(
        &self,
        project_id: &str,
        metric_name: &str,
        direction: MetricDirection,   // 'minimize' | 'maximize'
        top_k: u32,
    ) -> Result<Vec<RunRanking>>;

    /// 搜索 run（基于名称、tag、config、metric 值范围等）
    async fn search_runs(
        &self,
        project_id: &str,
        query: RunSearchQuery,
    ) -> Result<Vec<RunRanking>>;

    // ── Run 对比与分析 ────────────────────────────

    /// 结构化的 run 对比：返回统计表格而非原始序列
    async fn compare_runs(
        &self,
        run_ids: &[String],
        metric_names: &[String],
    ) -> Result<RunComparison>;

    /// 检测异常 run（NaN、发散、突变、plateau）
    async fn detect_anomaly(
        &self,
        run_id: &str,
        metric_names: Option<&[String]>,
    ) -> Result<Vec<AnomalyReport>>;

    /// 检测 project 下所有 run 的全局异常
    async fn detect_project_anomalies(
        &self,
        project_id: &str,
    ) -> Result<Vec<AnomalyReport>>;

    // ── 语义摘要 ──────────────────────────────────

    /// 生成 run 的可读摘要（markdown），LLM 可直接理解
    async fn summarize_run(&self, run_id: &str) -> Result<RunSummaryMarkdown>;

    /// 生成 project 的总览摘要（markdown）
    async fn summarize_project(&self, project_id: &str) -> Result<ProjectSummaryMarkdown>;

    // ── 时序数据（LLM 优化） ──────────────────────

    /// 获取降采样的 metric 序列，max_points 控制返回点数
    async fn get_metric_digest(
        &self,
        run_id: &str,
        metric_name: &str,
        max_points: u32,              // 默认 50，建议范围 20-200
    ) -> Result<MetricDigest>;

    /// 批量获取多个 metric 的 digest
    async fn get_metric_digests_batch(
        &self,
        run_id: &str,
        metric_names: &[String],
        max_points: u32,
    ) -> Result<Vec<MetricDigest>>;

    // ── 实验树 ────────────────────────────────────

    /// 获取 run 的实验谱系（父子树）
    async fn get_experiment_lineage(
        &self,
        run_id: &str,
        direction: LineageDirection,  // 'ancestors' | 'descendants' | 'both'
        max_depth: u32,
    ) -> Result<ExperimentTree>;

    // ── 假设与洞察 ────────────────────────────────

    async fn list_hypotheses(
        &self,
        project_id: &str,
        status: Option<HypothesisStatus>,
    ) -> Result<Vec<Hypothesis>>;

    async fn get_insights(
        &self,
        target: InsightTarget,       // SpecificRun(run_id) | Project(project_id)
        insight_type: Option<InsightType>,
        limit: Option<u32>,
    ) -> Result<Vec<Insight>>;

    async fn get_agent_decisions(
        &self,
        run_id: &str,
        decision_type: Option<DecisionType>,
        limit: Option<u32>,
    ) -> Result<Vec<AgentDecision>>;

    // ── AI 辅助推荐（可插拔 LLM 后端） ────────────

    /// 基于 project 历史，推荐下一个实验的配置
    async fn recommend_next_experiment(
        &self,
        project_id: &str,
        objective: &str,              // e.g. "minimize val_loss"
    ) -> Result<ExperimentRecommendation>;

    // ── 内省 ──────────────────────────────────────

    /// 返回 schema 信息，帮助 agent 理解数据结构
    async fn describe_schema(&self) -> Result<SchemaDescription>;

    /// 列出所有可用的 agent 工具（用于 LLM function calling / MCP）
    async fn list_tools(&self) -> Result<Vec<ToolDefinition>>;
}
```

### 13.2 核心返回类型

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRanking {
    pub run_id: String,
    pub run_name: String,
    pub status: RunStatus,
    pub rank: u32,
    pub metric_value: f64,
    pub metric_name: String,
    pub summary: RunSummarySnapshot,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunComparison {
    pub runs: Vec<RunComparisonRow>,
    pub metrics: Vec<String>,
    pub comparison_table: Vec<ComparisonRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonRow {
    pub run_id: String,
    pub run_name: String,
    pub metric_stats: HashMap<String, MetricStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub last: f64,
    pub best: f64,
    pub std: f64,
    pub total_steps: i64,
    pub trend: TrendDirection,       // improving, degrading, stable, volatile
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyReport {
    pub run_id: String,
    pub metric_name: String,
    pub anomaly_type: AnomalyType,   // nan_detected, divergence, spike, plateau, sudden_drop
    pub severity: AnomalySeverity,
    pub step: Option<i64>,
    pub description: String,         // 人类 + LLM 可读描述
    pub data_snippet: Vec<MetricPointCompact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDigest {
    pub run_id: String,
    pub metric_name: String,
    pub max_points: u32,
    pub total_points: u64,
    pub algorithm: String,           // 'lttb' | 'bucket_stats'
    pub stats: MetricStats,
    pub series: Vec<MetricPointCompact>,  // [{step, value}, ...]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPointCompact {
    pub step: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentTree {
    pub root_run_id: String,
    pub nodes: Vec<ExperimentTreeNode>,
    pub edges: Vec<ExperimentTreeEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummaryMarkdown {
    pub run_id: String,
    pub markdown: String,           // LLM 可直接插入 context
    pub token_estimate: u32,
}
```

### 13.3 DuckDB SQL 映射（Native 实现核心逻辑）

**`find_best_runs`**：
```sql
SELECT r.id, r.name, r.status,
       COALESCE(
           (SELECT MIN(value_f64) FROM dl.metric_points mp
            WHERE mp.run_id = r.id AND mp.metric_name = $1),
           json_extract_string(run_summary.metrics_summary, concat('$.', $1, '.min'))
       ) AS best_value
FROM dl.runs r
LEFT JOIN dl.run_summary ON r.id = dl.run_summary.run_id
WHERE r.project_id = $2 AND r.status = 'finished'
ORDER BY best_value ASC
LIMIT $3
```

**`compare_runs`**：
```sql
SELECT run_id, metric_name,
       MIN(value_f64) AS min_val, MAX(value_f64) AS max_val,
       AVG(value_f64) AS mean_val, STDDEV(value_f64) AS std_val,
       LAST(value_f64) AS last_val, COUNT(*) AS total_steps
FROM dl.metric_points
WHERE run_id IN (...) AND metric_name IN (...)
GROUP BY run_id, metric_name
ORDER BY run_id, metric_name
```

**`detect_anomaly`**：全部在 DuckDB SQL 中用窗口函数实现：
- NaN：`WHERE value_f64 IS NULL OR is_nan(value_f64)`
- 发散：滑动窗口 stddev > 历史均值的 3x
- plateau：最近 N 步 (max-min)/mean < 0.01
- spike：当前值与前一步差值 > 历史相邻差值均值的 5x

**`get_experiment_lineage`**：
```sql
WITH RECURSIVE ancestors AS (
    SELECT parent_run_id, run_id, reason, hypothesis_id, 1 AS depth
    FROM dl.experiment_lineage WHERE run_id = $1
    UNION ALL
    SELECT el.parent_run_id, el.run_id, el.reason, el.hypothesis_id, a.depth + 1
    FROM dl.experiment_lineage el
    JOIN ancestors a ON el.run_id = a.parent_run_id
    WHERE a.depth < $2
)
SELECT * FROM ancestors
```

### 13.4 Native vs Cloud 的 AgentToolInterface 差异

| 方法 | Native (DuckDB) | Cloud (ClickHouse) | 差异 |
|------|----------------|-------------------|------|
| `find_best_runs` | DuckDB SQL + DuckLake | ClickHouse SQL | SQL 方言差异 |
| `compare_runs` | DuckDB aggregate | ClickHouse aggregate | ClickHouse 的 `simpleState` 可能更快 |
| `detect_anomaly` | DuckDB window functions | ClickHouse `windowFunnel`/`array` 函数 | ClickHouse 有更丰富的异常检测函数 |
| `get_metric_digest` | DuckDB `lttb_sorted()` 扩展函数 | ClickHouse `largestTriangleThreeBuckets()` | 降采样在引擎内执行，数据不动 |
| `get_experiment_lineage` | DuckDB recursive CTE | PostgreSQL recursive CTE | 递归 CTE 都能做 |

**结论**：`AgentToolInterface` trait 本身引擎无关。Native 版本的 `get_metric_digest` 通过 duckdb-lttb 扩展在 DuckDB 引擎内执行 LTTB 降采样（`lttb_sorted` 跳过排序，利用 `metric_points` 按 step 天然有序），Cloud 版本通过 ClickHouse 的 `largestTriangleThreeBuckets()` 内置函数实现。输出格式化、token 预算控制全部在 Rust 端统一实现。

---

## 14. LLM 友好的输出格式与渐进式披露

### 14.1 渐进式披露（Progressive Disclosure）

行业最佳实践（2026 共识）：LLM 数据访问应遵循四级渐进式披露，从最紧凑到最详细，让 agent 自主决定需要多少粒度。

| 级别 | 格式 | Token 消耗 | 适用场景 |
|------|------|-----------|---------|
| **L1: 统计摘要** | 紧凑文本 | 2-5 tokens | "loss improved 15% over 50 epochs" |
| **L2: 关键点** | 紧凑文本 | 20-50 tokens | "loss: [2.1→0.3], plateau @ epoch 8, val_loss diverged after epoch 40" |
| **L3: 降采样序列** | JSON 数组 | 100-500 tokens | LTTB 降采样到 20-50 个代表点 |
| **L4: 原始数据** | Arrow/DataFrame | 500+ tokens | 仅当 agent 显式请求时，走 `QueryInterface` |

`AgentToolInterface` 方法默认返回 L1-L3 级数据。L4 级（原始数据）通过 `QueryInterface` 获取。

### 14.2 OutputFormat 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OutputFormat {
    /// 紧凑 JSON（默认）：LLM 最易解析，token 效率高
    Json,
    /// Markdown 表格：适合 LLM 阅读的格式化输出
    Markdown,
    /// 紧凑序列 JSON：仅用于 metric digest，返回 [[step, value], ...]
    /// token 效率最高的降采样序列格式
    CompactSeries,
    /// 结构化对象：返回 Rust struct 的 JSON 序列化
    Structured,
}
```

### 14.3 Token 预算控制

所有 `AgentToolInterface` 方法内部：
1. 序列化结果为 JSON string
2. 估算 token 数（启发式：`len_bytes / 3`，或集成 `tiktoken-rs`）
3. 如果超过默认预算（2000 tokens）：自动减少返回量（top_k、max_points、run 数量等）
4. 返回结果附带 `token_used` 和 `truncated` 标志

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse<T> {
    pub data: T,
    pub meta: AgentResponseMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponseMeta {
    pub token_estimate: u32,
    pub truncated: bool,
    pub output_format: OutputFormat,
    pub query_duration_ms: u64,
}
```

### 14.4 CompactSeries 输出示例

**Structured 格式**（人类阅读友好）：
```json
{
  "run_id": "abc123",
  "metric_name": "train/loss",
  "stats": {"min": 0.12, "max": 5.68, "mean": 1.23, "last": 0.35},
  "series": [{"step": 0, "value": 5.68}, {"step": 100, "value": 3.46}, ...]
}
```

**CompactSeries 格式**（LLM token 最优，节省约 60%）：
```json
{"r":"abc123","m":"train/loss","s":{"n":0.12,"x":5.68,"a":1.23,"l":0.35},
 "p":[[0,5.68],[100,3.46],[200,2.34],[300,1.56],[400,0.89],[500,0.45],[600,0.35]]}
```

### 14.5 Schema 自省

Agent 首次使用时可通过 `describe_schema()` 了解整个数据结构，然后自主决定调用哪些工具：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDescription {
    pub version: String,
    pub tables: Vec<TableDescription>,
    pub entity_relationships: Vec<EntityRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDescription {
    pub name: String,
    pub storage_type: String,     // 'catalog' | 'parquet'
    pub description: String,
    pub columns: Vec<ColumnDescription>,
    pub row_count_estimate: u64,
}
```

---

## 15. Tool-Calling 接口（LLM function calling）

### 15.1 设计目标

让 LLM Agent（OpenAI/Anthropic/Gemini）通过标准 function calling 机制直接调用 PulseOn。`pulseon.agent` 模块负责：
1. 生成兼容各 LLM 平台的 tool definitions（JSON schema）
2. 接收 LLM 的 tool call 请求并执行
3. 返回 LLM 可消费的结果

### 15.2 三种 Tool Schema 格式

行业现状（2026）存在三种主流 tool-calling schema 格式，PulseOn 同时支持：

| 格式 | 使用者 | schema 字段名 | 包装方式 |
|------|--------|-------------|---------|
| **OpenAI** | GPT-4o, GPT-5 | `parameters` | `{"type": "function", "function": {...}}` |
| **Anthropic** | Claude 4, Claude Opus | `input_schema` | 顶层对象 |
| **MCP** | Claude Code, Cursor, Codex | `inputSchema` (camelCase) | MCP protocol 规范 |

```rust
/// Tool schema 导出格式
pub enum ToolSchemaFormat {
    OpenAI,     // {"type": "function", "function": {"name", "description", "parameters"}}
    Anthropic,  // {"name", "description", "input_schema"}
    Mcp,        // {"name", "description", "inputSchema"}
}
```

### 15.3 Tool Definition 结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,                   // verb-first snake_case: "find_best_runs"
    pub description: String,            // 包含 WHEN to call AND when NOT to call
    pub parameters: ToolParameters,     // JSON Schema for parameters
    pub returns: ToolReturnDescription, // 返回格式说明
}
```

### 15.4 默认 Tool Set

PulseOn 为 AI Agent 提供的默认 tool set（10 个核心工具，符合"限制 active tools 到 10-20"的最佳实践）：

| 工具名 | 描述 | 返回格式 |
|--------|------|---------|
| `find_best_runs` | 按 metric 排名查找 top-k run | JSON |
| `search_runs` | 按名称/tag/metric 范围搜索 run | JSON |
| `compare_runs` | 多 run 结构化对比（统计表） | JSON |
| `detect_anomaly` | 检测 run 异常（NaN/发散/spike/plateau） | JSON |
| `summarize_run` | 生成 run 的 markdown 摘要 | Markdown |
| `summarize_project` | 生成 project 的 markdown 总览 | Markdown |
| `get_metric_digest` | 获取降采样 metric 序列（LTTB） | JSON (CompactSeries) |
| `get_experiment_lineage` | 获取实验谱系树 | JSON |
| `list_hypotheses` | 列出 project 的假设及状态 | JSON |
| `get_insights` | 获取 run/project 的洞察 | JSON |

**工具描述最佳实践**（2026 共识）：
- 名称：verb-first snake_case — `find_best_runs`，不是 `metric_query`
- 描述：包含何时调用 AND 何时不调用 — "Use this for training metrics. Do NOT use for trace data."
- 参数描述：包含示例 — "Project name, e.g. 'image-classifier'"
- 使用 enum 约束可选值 — `"enum": ["minimize", "maximize"]`
- 返回结构化 JSON，包含 summary + data，不只是 raw data

### 15.5 工具执行调度

```rust
/// 通用工具执行入口——接收 LLM function call 的 tool_name + arguments，
/// 分发到对应的 AgentToolInterface 方法
pub async fn execute_tool(
    agent: &dyn AgentToolInterface,
    tool_name: &str,
    arguments: serde_json::Value,
    output_format: OutputFormat,
) -> Result<AgentToolResult> {
    match tool_name {
        "find_best_runs" => { /* parse args, call agent.find_best_runs, format result */ }
        "summarize_run" => { /* ... */ }
        "compare_runs" => { /* ... */ }
        // ... 其他方法
        _ => Err(AgentError::UnknownTool(tool_name.into())),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolResult {
    pub content: String,          // JSON string 或 markdown string
    pub content_type: String,     // "application/json" | "text/markdown"
    pub token_estimate: u32,
    pub truncated: bool,
}
```

### 15.6 Python SDK 中的 OpenAI 集成示例

```python
import pulseon
import openai

client = pulseon.init("./runs")
agent = client.agent

# 获取兼容 OpenAI function calling 的 tool definitions
tools = agent.get_tool_definitions(format="openai")

response = openai.chat.completions.create(
    model="gpt-4o",
    messages=[
        {"role": "system", "content": "You are a ML research assistant. "
                                       "Use PulseOn tools to analyze experiments."},
        {"role": "user", "content": "Find the best run in 'image-classifier' "
                                     "by val_loss and summarize it."},
    ],
    tools=tools,
    tool_choice="auto",
)

# 处理 tool call
if response.choices[0].message.tool_calls:
    for tool_call in response.choices[0].message.tool_calls:
        result = agent.execute_tool(
            tool_call.function.name,
            tool_call.function.arguments,  # JSON string
        )
        messages.append({
            "role": "tool",
            "tool_call_id": tool_call.id,
            "content": result.content,
        })
```

---

## 16. MCP Server（Agent 的通用接口）

### 16.1 为什么需要 MCP Server

**MCP（Model Context Protocol）正在成为 AI Agent 访问外部工具的通用标准。** 行业验证：

- **W&B 已有 GA MCP Server**（[wandb/wandb-mcp-server](https://github.com/wandb/wandb-mcp-server)），20 个工具，2026 年 5 月正式发布，是实验追踪领域 agent 接口的行业基准
- **多个 DuckDB MCP Server 已存在**（mustafahasankhan/duckdb-mcp-server 等），验证了 DuckDB + MCP 的可行性
- **Claude Code、Cursor、Codex 等主流 agent IDE 原生支持 MCP**——agent 无需编写自定义集成代码即可使用 MCP server
- **SwanLab 和 MLflow 均无 MCP server**——这是 PulseOn 的差异化机会

**设计决策**：PulseOn 在 Native 版本中内置 MCP server，使任何支持 MCP 的 agent 客户端（Claude Code、Cursor、Codex、自定义 agent）都能零集成地访问实验数据。MCP server 与 Python SDK agent 模块共享同一 `AgentToolInterface` 实现，只是暴露方式不同。

### 16.2 MCP Server 架构

```
┌─────────────────────────────────────────────────────────────┐
│                    Agent 客户端                               │
│  (Claude Code / Cursor / Codex / 自定义 agent)               │
└──────────────────────────┬──────────────────────────────────┘
                           │ MCP Protocol (stdio / SSE / HTTP)
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              PulseOn MCP Server (Rust)                       │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  MCP Protocol Layer                                  │    │
│  │  - JSON-RPC over stdio / SSE                         │    │
│  │  - tool list / tool call / resource read             │    │
│  └───────────────────────┬─────────────────────────────┘    │
│                          │                                   │
│  ┌───────────────────────▼─────────────────────────────┐    │
│  │  Tool Dispatcher                                     │    │
│  │  (maps MCP tool names → AgentToolInterface methods) │    │
│  └───────────────────────┬─────────────────────────────┘    │
│                          │                                   │
│  ┌───────────────────────▼─────────────────────────────┐    │
│  │  AgentToolInterface impl (DuckDBAgentTools)          │    │
│  │  - find_best_runs, compare_runs, summarize_run, ...  │    │
│  │  - LTTB downsampling, anomaly detection              │    │
│  └───────────────────────┬─────────────────────────────┘    │
│                          │                                   │
│  ┌───────────────────────▼─────────────────────────────┐    │
│  │  DuckDB + DuckLake (existing architecture)           │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

### 16.3 MCP Server 实现

MCP server 作为 PulseOn 的一个独立入口点运行，复用 `AgentToolInterface` 的全部实现：

```rust
// src/mcp/server.rs

use crate::compute::AgentToolInterface;
use crate::engine::PulseOnClient;

/// PulseOn MCP Server
/// 
/// 启动方式：
///   1. CLI: `pulseon mcp serve --path ./runs`
///   2. Python: `pulseon.mcp.serve("./runs")`
///   3. 配置文件：agent IDE 的 MCP 配置指向 pulseon mcp 二进制
pub struct PulseOnMcpServer {
    client: Arc<PulseOnClient>,
    agent: Arc<dyn AgentToolInterface>,
}

impl PulseOnMcpServer {
    /// 启动 MCP server（stdio 模式，供 Claude Code / Cursor 等使用）
    pub async fn serve_stdio(&self) -> Result<()> {
        // MCP protocol: JSON-RPC over stdio
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        
        // 注册 tools
        let tools = self.agent.list_tools().await?;
        
        // 处理 JSON-RPC 请求循环
        loop {
            let request = read_jsonrpc_request(&stdin).await?;
            match request.method.as_str() {
                "tools/list" => {
                    let response = self.build_tool_list_response(&tools);
                    write_jsonrpc_response(&stdout, request.id, response).await?;
                }
                "tools/call" => {
                    let tool_name = request.params["name"].as_str().unwrap();
                    let arguments = request.params["arguments"].clone();
                    let result = self.agent.execute_tool(tool_name, arguments).await?;
                    write_jsonrpc_response(&stdout, request.id, result).await?;
                }
                "resources/read" => {
                    // 暴露 run/report/insight 作为 MCP resources
                    // ...
                }
                _ => {
                    write_jsonrpc_error(&stdout, request.id, "method_not_found").await?;
                }
            }
        }
    }
    
    fn build_tool_list_response(&self, tools: &[ToolDefinition]) -> serde_json::Value {
        // MCP tool schema format: {name, description, inputSchema (camelCase)}
        serde_json::json!({
            "tools": tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.parameters  // MCP uses camelCase "inputSchema"
            })).collect::<Vec<_>>()
        })
    }
}
```

### 16.4 MCP Tool Schema 格式

MCP 使用 `inputSchema`（camelCase），与 OpenAI 的 `parameters` 和 Anthropic 的 `input_schema` 不同：

```json
{
  "name": "find_best_runs",
  "description": "Find the top-K runs in a project ranked by a specific metric. Use this when you need to identify the best performing experiments. Do NOT use for querying trace data.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "project_id": {"type": "string", "description": "Project ID or name"},
      "metric_name": {"type": "string", "description": "Metric to rank by, e.g. 'val/loss'"},
      "direction": {"type": "string", "enum": ["minimize", "maximize"]},
      "top_k": {"type": "integer", "description": "Number of top runs (default: 5, max: 20)"}
    },
    "required": ["project_id", "metric_name", "direction"]
  }
}
```

### 16.5 MCP Resources（只读数据暴露）

除了 tools（可执行操作），MCP 还支持 resources（只读数据）。PulseOn 暴露以下 resources：

| Resource URI | 描述 | 格式 |
|-------------|------|------|
| `pulseon://projects` | 所有 project 列表 | JSON |
| `pulseon://project/{id}/summary` | Project 总览 | Markdown |
| `pulseon://run/{id}/summary` | Run 摘要 | Markdown |
| `pulseon://run/{id}/metrics/digest` | Run 的所有 metric digest | JSON |
| `pulseon://project/{id}/insights` | Project 的洞察列表 | JSON |
| `pulseon://project/{id}/hypotheses` | Project 的假设列表 | JSON |
| `pulseon://project/{id}/reports/{rid}` | 特定报告全文 | Markdown |

### 16.6 Agent IDE 配置

用户在 Claude Code / Cursor / Codex 的 MCP 配置中添加 PulseOn：

```json
// Claude Code MCP 配置 (.claude/mcp.json)
{
  "mcpServers": {
    "pulseon": {
      "command": "pulseon",
      "args": ["mcp", "serve", "--path", "./my_training_runs"]
    }
  }
}
```

```python
# Python 中启动 MCP server（用于自定义 agent 集成）
import pulseon

# 方式 1: 直接启动 stdio MCP server
pulseon.mcp.serve("./my_training_runs")

# 方式 2: 获取 MCP 配置供 agent 框架使用
config = pulseon.mcp.get_config("./my_training_runs")
# config = {"command": "pulseon", "args": ["mcp", "serve", "--path", "./my_training_runs"]}
```

### 16.7 W&B Skills 层的启示

W&B 在 MCP tools 之上提供了 [Skills 层](https://github.com/wandb/skills)——工作流级别的 agent 指导（"opinionated workflows"）。例如 `wandb-primary` skill 告诉 agent 何时用 MCP tools、何时用 SDK。

**PulseOn 的对应设计**：未来可提供 `pulseon.skills` 模块，包含预置的 auto-research 工作流模板：
- `skill_basic_research`：基础实验分析工作流
- `skill_hyperparameter_search`：超参搜索工作流
- `skill_failure_analysis`：训练失败诊断工作流

v1 阶段先实现 MCP tools + Python `AutoResearchAgent`，Skills 层作为 v2 增强。

---

## 17. Auto-Research 工作流设计

### 17.1 端到端流程

```
┌────────────────────────────────────────────────────────────────────┐
│                     Auto Research Main Loop                         │
│                                                                     │
│  ┌─ 1. Context Gathering ──────────────────────────────────────┐   │
│  │  • project = agent.summarize_project(project_id)              │   │
│  │  • hypotheses = agent.list_hypotheses(project_id)             │   │
│  │  • insights = agent.get_insights(project_id)                  │   │
│  │  • best_runs = agent.find_best_runs(project_id, "val_loss")   │   │
│  │  → LLM: "Here's the current state of research..."            │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     ▼                                                               │
│  ┌─ 2. Hypothesis Generation ──────────────────────────────────┐   │
│  │  • recommendation = agent.recommend_next_experiment(...)      │   │
│  │  • OR LLM proposes new hypothesis based on context            │   │
│  │  • hypothesis_id = client.create_hypothesis(...)              │   │
│  │  → LLM: "I hypothesize that increasing LR to 0.01 will..."   │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     ▼                                                               │
│  ┌─ 3. Experiment Launch ──────────────────────────────────────┐   │
│  │  • run = client.create_run(config=proposed_config,             │   │
│  │           hypothesis_id=hypothesis_id)                         │   │
│  │  • agent.log_agent_decision("launch_new_run", reasoning=...)  │   │
│  │  • Launch training script (subprocess / remote)               │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     ▼                                                               │
│  ┌─ 4. Real-time Monitoring ───────────────────────────────────┐   │
│  │  Loop while run.status == 'running':                          │   │
│  │  • status = agent.get_run_status(run_id)                       │   │
│  │  • latest = agent.get_recent_metrics(run_id, since_step=N)    │   │
│  │  • anomalies = agent.detect_anomaly(run_id)                    │   │
│  │  • if anomaly → LLM decides: stop / continue / tweak          │   │
│  │  • if convergence → break                                     │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     ▼                                                               │
│  ┌─ 5. Post-run Analysis ──────────────────────────────────────┐   │
│  │  • comparison = agent.compare_runs([new_run, baseline, ...])   │   │
│  │  • digest = agent.get_metric_digest(run_id, "val_loss")        │   │
│  │  • anomalies = agent.detect_anomaly(run_id)                    │   │
│  │  → LLM: "The new experiment improved val_loss by 12%..."      │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     ▼                                                               │
│  ┌─ 6. Insight & Report Generation ────────────────────────────┐   │
│  │  • agent.add_insight(run_id, type="finding", ...)             │   │
│  │  • agent.update_hypothesis(hypothesis_id, status="confirmed") │   │
│  │  • report = agent.generate_report(project_id, run_ids=[...])  │   │
│  │  → LLM: "Key findings: ... Next experiment: ..."             │   │
│  └──────────────────────────────────────────────────────────────┘   │
│     │                                                               │
│     └──→ Loop back to step 2 with new context                       │
└────────────────────────────────────────────────────────────────────┘
```

### 17.2 Python Auto-Research Agent 实现

```python
import pulseon
import openai
import json
import time

class AutoResearchAgent:
    """使用 PulseOn + OpenAI 的 auto research agent"""

    def __init__(self, pulseon_path: str, openai_model: str = "gpt-4o"):
        self.client = pulseon.init(pulseon_path)
        self.agent = self.client.agent
        self.model = openai_model
        self.tools = self.agent.get_tool_definitions(format="openai")
        self.system_prompt = """You are an AI research scientist.
Your goal is to autonomously improve a machine learning model by:
1. Reviewing past experiments
2. Proposing hypotheses about what will improve the model
3. Analyzing experiment results
4. Generating insights and next steps
Use PulseOn tools to query the experiment database. Be methodical and data-driven."""

    def run_auto_research_loop(self, project_id: str, objective: str, max_iterations: int = 3):
        """执行 auto research 主循环"""
        for iteration in range(max_iterations):
            # Step 1: Gather context
            context = self._gather_context(project_id)
            # Step 2: Propose hypothesis
            hypothesis = self._propose_experiment(project_id, objective, context)
            if hypothesis is None:
                break
            # Step 3: Launch experiment
            run = self._launch_experiment(project_id, hypothesis)
            # Step 4: Monitor
            self._monitor_experiment(run['run_id'])
            # Step 5: Analyze
            analysis = self._analyze_experiment(run['run_id'], hypothesis.get('baseline_run_id'))
            # Step 6: Generate insights
            self._generate_insights(run['run_id'], hypothesis['hypothesis_id'], analysis)
        # Final report
        report = self.agent.generate_report(project_id)
        return report

    def _gather_context(self, project_id: str) -> dict:
        summary = self.agent.summarize_project(project_id)
        hypotheses = self.agent.list_hypotheses(project_id)
        insights = self.agent.get_insights(project_id=project_id)
        best_5 = self.agent.find_best_runs(project_id, "val_loss", "minimize", top_k=5)
        return {
            "summary": summary.markdown,
            "hypotheses": [h.to_dict() for h in hypotheses],
            "insights": [i.to_dict() for i in insights],
            "best_runs": [r.to_dict() for r in best_5],
        }

    def _propose_experiment(self, project_id, objective, context):
        messages = [
            {"role": "system", "content": self.system_prompt},
            {"role": "user", "content": f"Project: {project_id}\nObjective: {objective}\n"
                                         f"Context:\n{json.dumps(context, indent=2)}\n\n"
                                         "Propose ONE hypothesis and experiment. Return JSON with: "
                                         "hypothesis_title, hypothesis_description, "
                                         "independent_variable, expected_outcome, "
                                         "config_changes, baseline_run_id"},
        ]
        response = openai.chat.completions.create(
            model=self.model, messages=messages,
            response_format={"type": "json_object"},
        )
        proposal = json.loads(response.choices[0].message.content)
        hypothesis_id = self.client.create_hypothesis(
            project_id=project_id,
            title=proposal['hypothesis_title'],
            description=proposal['hypothesis_description'],
            independent_variable=proposal['independent_variable'],
            expected_outcome=proposal['expected_outcome'],
            source_type="agent", agent_model=self.model,
        )
        proposal['hypothesis_id'] = hypothesis_id
        return proposal

    def _launch_experiment(self, project_id, hypothesis):
        baseline_config = {}
        if hypothesis.get('baseline_run_id'):
            baseline = self.client.get_run(hypothesis['baseline_run_id'])
            baseline_config = baseline.config
        new_config = {**baseline_config, **hypothesis.get('config_changes', {})}
        run = self.client.create_run(
            project=project_id,
            name=f"auto-research-{hypothesis['hypothesis_id'][:8]}",
            config=new_config,
            hypothesis_id=hypothesis['hypothesis_id'],
            tags=["auto-research"],
        )
        if hypothesis.get('baseline_run_id'):
            self.client.add_experiment_lineage(
                run_id=run.id, parent_run_id=hypothesis['baseline_run_id'],
                hypothesis_id=hypothesis['hypothesis_id'],
                reason=hypothesis['hypothesis_title'],
                diff_summary=json.dumps(hypothesis.get('config_changes', {})),
            )
        self.agent.log_agent_decision(
            run_id=run.id, decision_type="launch_new_run",
            reasoning=f"Testing hypothesis: {hypothesis['hypothesis_title']}",
        )
        self._run_training(run, new_config)
        return {"run_id": run.id, "hypothesis_id": hypothesis['hypothesis_id']}

    def _monitor_experiment(self, run_id):
        last_step = 0
        plateau_count = 0
        while True:
            time.sleep(30)
            status = self.agent.get_run_status(run_id)
            if status['status'] != 'running':
                break
            recent = self.agent.get_recent_metrics(run_id, since_step=last_step)
            if recent['steps']:
                last_step = recent['steps'][-1]
            anomalies = self.agent.detect_anomaly(run_id)
            for a in anomalies:
                if a['severity'] == 'critical':
                    self.agent.log_agent_decision(
                        run_id=run_id, decision_type="mark_anomaly",
                        reasoning=f"Critical anomaly: {a['description']}",
                    )
            if recent.get('val_loss', {}).get('trend') == 'stable':
                plateau_count += 1
            else:
                plateau_count = 0
            if plateau_count >= 3:
                self.agent.log_agent_decision(
                    run_id=run_id, decision_type="stop_training",
                    reasoning=f"Loss plateaued for {plateau_count * 30}s",
                )
                self.client.stop_run(run_id)
                break

    def _analyze_experiment(self, run_id, baseline_run_id=None):
        digest = self.agent.get_metric_digest(run_id, "val_loss", max_points=100)
        comparison = None
        if baseline_run_id:
            comparison = self.agent.compare_runs([run_id, baseline_run_id], ["val_loss"])
        summary = self.agent.summarize_run(run_id)
        anomalies = self.agent.detect_anomaly(run_id)
        return {"summary": summary, "val_loss_stats": digest.stats,
                "comparison": comparison, "anomalies": anomalies}

    def _generate_insights(self, run_id, hypothesis_id, analysis):
        messages = [
            {"role": "system", "content": "Generate concise, actionable insights from experiment results."},
            {"role": "user", "content": json.dumps(analysis, default=str, indent=2)},
        ]
        response = openai.chat.completions.create(
            model=self.model, messages=messages,
            response_format={"type": "json_object"},
        )
        insights_data = json.loads(response.choices[0].message.content)
        for insight in insights_data.get('insights', []):
            self.agent.add_insight(
                run_id=run_id, ins_type=insight.get('type', 'finding'),
                title=insight['title'], content=insight['content'],
                source_type="agent",
            )
        self.client.update_hypothesis(hypothesis_id, status="confirmed")

    def _run_training(self, run, config):
        """替换为实际训练代码或 subprocess 调用"""
        import random
        lr = config.get('learning_rate', 0.001)
        for step in range(1000):
            train_loss = 2.0 * (0.95 ** (step * lr * 100)) + random.random() * 0.1
            val_loss = train_loss * 1.1 + random.random() * 0.05
            run.log_metrics({"train/loss": train_loss, "val/loss": val_loss}, step=step)
            time.sleep(0.01)
        run.finish()


# 使用
if __name__ == "__main__":
    agent = AutoResearchAgent("./auto_research_runs")
    report = agent.run_auto_research_loop(
        project_id="image-classifier",
        objective="minimize val_loss",
        max_iterations=3,
    )
```

---

## 18. 实时监控（Agent 视角）

### 18.1 Native 模式约束

Native 模式没有服务端，所有操作在同一进程内。Agent 通过**轮询（polling）**监控 run 状态。如果 agent 和训练脚本在不同进程，通过读取相同的 DuckLake catalog 文件共享数据（SQLite WAL mode 允许多读单写）。

### 18.2 监控接口

```rust
/// Agent-facing real-time monitoring
#[async_trait::async_trait]
pub trait AgentMonitor: Send + Sync {
    /// 获取 run 当前状态 + 最新指标快照
    async fn get_run_status(&self, run_id: &str) -> Result<RunStatusSnapshot>;

    /// 增量获取指标：只返回 since_step 之后的新数据点
    async fn get_recent_metrics(
        &self,
        run_id: &str,
        since_step: i64,
        metric_names: Option<&[String]>,
    ) -> Result<RecentMetricsSnapshot>;

    /// 获取 run 的实时指标摘要
    async fn get_live_summary(&self, run_id: &str) -> Result<LiveSummary>;

    /// 注册告警条件（本地回调模式），返回 watcher_id
    async fn watch_condition(
        &self,
        run_id: &str,
        condition: AlertCondition,
        callback: Box<dyn Fn(AlertEvent) + Send>,
    ) -> Result<String>;

    /// 取消告警
    async fn unwatch(&self, watcher_id: &str) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatusSnapshot {
    pub run_id: String,
    pub status: RunStatus,
    pub current_step: i64,
    pub latest_metrics: HashMap<String, f64>,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertCondition {
    pub metric_name: String,
    pub condition_type: AlertType,       // no_improvement, value_above, value_below, nan_detected
    pub threshold: f64,
    pub window_steps: i64,
}
```

### 18.3 watch_condition 实现

Native 模式下 `watch_condition` 是轻量后台轮询任务：

```rust
impl AgentMonitor for DuckDBAgentTools {
    async fn watch_condition(
        &self,
        run_id: &str,
        condition: AlertCondition,
        callback: Box<dyn Fn(AlertEvent) + Send>,
    ) -> Result<String> {
        let conn = self.conn.clone();
        let run_id = run_id.to_string();
        let watcher_id = uuid::Uuid::new_v4().to_string();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                // 查询最近 window_steps 的 metric 值
                // 检查条件是否触发
                // 如果触发，调用 callback(AlertEvent { ... })
            }
        });
        Ok(watcher_id)
    }
}
```

**Python 使用**：

```python
def on_loss_diverging(event):
    print(f"ALERT: {event.metric_name} = {event.current_value} at step {event.step}")
    agent.log_agent_decision(
        run_id=event.run_id, decision_type="mark_anomaly",
        reasoning=f"Loss diverging: {event.current_value}",
    )

watcher_id = agent.watch_condition(
    run_id=run.id,
    condition=AlertCondition(
        metric_name="train/loss",
        condition_type="value_above",
        threshold=10.0,
        window_steps=10,
    ),
    callback=on_loss_diverging,
)
```

### 18.4 跨进程监控

当 agent 和训练脚本在不同 Python 进程时，两者通过相同的 DuckLake catalog 文件共享数据。Agent 进程使用只读 DuckDB connection，训练进程写入——SQLite WAL mode 天然兼容。

---

## 19. Python SDK：agent 模块

### 19.1 模块结构

```python
# pulseon/agent/__init__.py

class Agent:
    """PulseOn AI Agent 接口——为 LLM 提供语义化的实验数据访问。
    与 developer-facing API (client.query_*) 互补。
    """

    def __init__(self, client: "pulseon.Client"):
        self._client = client
        self._rust_agent = client._rust.get_agent()

    # ── 工具定义 ────────────────────────────────────

    def get_tool_definitions(self, format: str = "openai") -> list[dict]:
        """返回 LLM function calling 兼容的工具定义。
        format: 'openai' | 'anthropic' | 'gemini' | 'mcp'
        """
        ...

    def execute_tool(self, tool_name: str, arguments: str | dict) -> AgentToolResult:
        """执行 LLM 请求的 tool call，返回可直接传回 LLM 的结果。"""
        ...

    # ── 语义查询 ────────────────────────────────────

    def find_best_runs(self, project: str, metric: str,
                       direction: str = "minimize", top_k: int = 5) -> list[RunRanking]: ...
    def search_runs(self, project: str, query: str | dict, limit: int = 10) -> list[RunRanking]: ...
    def compare_runs(self, run_ids: list[str], metrics: list[str]) -> RunComparison: ...
    def detect_anomaly(self, run_id: str, metrics: list[str] | None = None) -> list[AnomalyReport]: ...
    def summarize_run(self, run_id: str) -> RunSummaryMarkdown: ...
    def summarize_project(self, project: str) -> ProjectSummaryMarkdown: ...
    def get_metric_digest(self, run_id: str, metric: str, max_points: int = 50) -> MetricDigest: ...
    def get_metric_digests_batch(self, run_id: str, metrics: list[str], max_points: int = 50) -> list[MetricDigest]: ...
    def get_experiment_lineage(self, run_id: str, direction: str = "both", max_depth: int = 5) -> ExperimentTree: ...
    def list_hypotheses(self, project: str, status: str | None = None) -> list[Hypothesis]: ...
    def get_insights(self, run_id: str | None = None, project_id: str | None = None,
                     ins_type: str | None = None, limit: int = 20) -> list[Insight]: ...
    def get_agent_decisions(self, run_id: str, decision_type: str | None = None, limit: int = 20) -> list[AgentDecision]: ...
    def recommend_next_experiment(self, project: str, objective: str) -> ExperimentRecommendation: ...
    def describe_schema(self) -> SchemaDescription: ...

    # ── 写入操作 ────────────────────────────────────

    def add_insight(self, run_id: str | None = None, project_id: str | None = None,
                    ins_type: str = "finding", title: str = "", content: str = "",
                    evidence: dict | None = None, source_type: str = "agent",
                    severity: str = "info") -> str: ...
    def log_agent_decision(self, run_id: str, decision_type: str, reasoning: str,
                           action_taken: str | None = None, data_snapshot: dict | None = None) -> str: ...
    def generate_report(self, project: str, run_ids: list[str] | None = None,
                        template: str | None = None) -> Report: ...

    # ── 实时监控 ────────────────────────────────────

    def get_run_status(self, run_id: str) -> RunStatusSnapshot: ...
    def get_recent_metrics(self, run_id: str, since_step: int,
                           metrics: list[str] | None = None) -> RecentMetricsSnapshot: ...
    def get_live_summary(self, run_id: str) -> LiveSummary: ...
    def watch_condition(self, run_id: str, condition: AlertCondition,
                        callback: Callable[[AlertEvent], None]) -> str: ...
    def unwatch(self, watcher_id: str) -> None: ...
```

### 19.2 入口

```python
# pulseon/__init__.py
class Client:
    @property
    def agent(self) -> "Agent":
        """返回 AI Agent 接口。延迟创建，首次访问时初始化。"""
        if self._agent is None:
            from pulseon.agent import Agent
            self._agent = Agent(self)
        return self._agent
```

使用方式：

```python
import pulseon

client = pulseon.init("./runs")

# Agent 方式（LLM 友好）
best = client.agent.find_best_runs("image-classifier", "val_loss", "minimize", top_k=5)
summary = client.agent.summarize_run(best[0].run_id)

# Developer 方式（原始数据）
df = client.query_metric_series(run_ids=[best[0].run_id], metrics=["val_loss"])
```

### 19.3 MCP 模块

```python
# pulseon/mcp/__init__.py

def serve(path: str, transport: str = "stdio"):
    """启动 MCP server，供 Claude Code / Cursor / Codex 等 agent IDE 使用。
    
    Args:
        path: PulseOn 数据路径
        transport: 'stdio'（默认，供 IDE 集成）或 'sse'（供远程 agent）
    """
    ...

def get_config(path: str) -> dict:
    """返回 MCP server 配置，供 agent 框架使用。
    
    Returns:
        {"command": "pulseon", "args": ["mcp", "serve", "--path", path]}
    """
    ...
```

---

## 20. Rust 模块扩展

### 20.1 更新后的模块结构

在 Part I 的模块结构基础上新增：

```
src/
├── model/
│   ├── hypothesis.rs            # NEW: Hypothesis, HypothesisStatus
│   ├── lineage.rs               # NEW: ExperimentLineage, LineageDirection
│   ├── agent_decision.rs        # NEW: AgentDecision, DecisionType
│   ├── insight.rs               # NEW: Insight, InsightType, InsightTarget
│   ├── report.rs                # NEW: Report
│   ├── digest.rs                # NEW: MetricDigest, MetricPointCompact, BucketStats
│   └── agent_types.rs           # NEW: AnomalyReport, RunRanking, RunComparison, etc.
│
├── catalog/
│   ├── trait.rs                 # +9 new methods (hypothesis/lineage/insight CRUD)
│   └── ducklake_impl.rs         # +DuckLake table creation for new tables
│
├── compute/
│   ├── agent_tools.rs           # NEW: AgentToolInterface trait
│   ├── agent_duckdb_impl.rs     # NEW: DuckDBAgentTools impl
│   ├── agent_cloud_impl.rs      # NEW: skeleton for ClickHouse
│   ├── semantic.rs              # NEW: semantic SQL builders
│   └── output.rs                # NEW: OutputFormat, token estimation
│
├── engine/
│   ├── agent.rs                 # NEW: agent orchestration, execute_tool, WatchManager
│   └── digest.rs                # NEW: LTTB, bucket stats, adaptive downsampling
│
├── mcp/                         # NEW: MCP server
│   ├── mod.rs
│   ├── server.rs                # MCP protocol (JSON-RPC over stdio/SSE)
│   ├── tools.rs                 # MCP tool schema export (inputSchema format)
│   └── resources.rs             # MCP resources (read-only data)
│
└── sdk/
    ├── agent.rs                 # NEW: #[pyclass] Agent
    └── tools.rs                 # NEW: OpenAI/Anthropic/Gemini tool schema export
```

### 20.2 新增 CatalogLayer 方法

```rust
// 在 CatalogLayer trait 中新增：

// ── Hypothesis ──────────────────────────────
async fn create_hypothesis(&self, hypothesis: NewHypothesis) -> Result<Hypothesis>;
async fn get_hypothesis(&self, id: &str) -> Result<Hypothesis>;
async fn list_hypotheses(&self, project_id: &str, status: Option<HypothesisStatus>) -> Result<Vec<Hypothesis>>;
async fn update_hypothesis_status(&self, id: &str, status: HypothesisStatus) -> Result<()>;

// ── Experiment Lineage ──────────────────────
async fn add_experiment_lineage(&self, lineage: NewLineage) -> Result<ExperimentLineage>;
async fn get_experiment_lineage(&self, run_id: &str, direction: LineageDirection, max_depth: u32) -> Result<Vec<ExperimentLineage>>;

// ── Agent Decision ──────────────────────────
async fn log_agent_decision(&self, decision: NewAgentDecision) -> Result<AgentDecision>;
async fn get_agent_decisions(&self, run_id: &str, decision_type: Option<DecisionType>, limit: Option<u32>) -> Result<Vec<AgentDecision>>;

// ── Insight ─────────────────────────────────
async fn add_insight(&self, insight: NewInsight) -> Result<Insight>;
async fn get_insights(&self, target: InsightTarget, insight_type: Option<InsightType>, limit: Option<u32>) -> Result<Vec<Insight>>;
async fn update_insight(&self, id: &str, status: &str) -> Result<()>;

// ── Report ──────────────────────────────────
async fn create_report(&self, report: NewReport) -> Result<Report>;
async fn get_report(&self, id: &str) -> Result<Report>;

// ── Metric Digest ───────────────────────────
async fn get_cached_digest(&self, run_id: &str, metric_name: &str, max_points: u32, algorithm: &str) -> Result<Option<MetricDigest>>;
async fn cache_digest(&self, digest: MetricDigest) -> Result<()>;
```

### 20.3 新增依赖

```toml
# 新增 Cargo.toml dependencies

# UUID 生成
uuid = { version = "1", features = ["v4"] }

# MCP server（JSON-RPC over stdio）
# 选项 A: 使用 rmcp crate（Rust MCP SDK）
rmcp = "0.1"
# 选项 B: 手动实现 JSON-RPC（更轻量，无额外依赖）
# tokio 已有

# Token 估算（可选）
# tiktoken-rs = "0.6"
```

---

## 21. 降采样策略（上下文效率）

### 21.1 为什么降采样是 AI Native 的关键

LLM 上下文窗口有限（8K-128K tokens），每个 token 都有成本。100k 个 `(step, value)` 数据点即使压缩也远超预算。LLM 分析训练曲线需要的是**形状**而非每一个点。

**行业验证**：LTTB（Largest Triangle Three Buckets）是时序降采样的行业标准，被 matplotlib、plotly、Grafana、Apache ECharts、ClickHouse（`largestTriangleThreeBuckets()` 函数）、TimescaleDB 广泛使用。源自 Sveinn Steinarsson 2013 年的 MSc 论文。

### 21.2 实现选型：duckdb-lttb 扩展（非 Rust 端实现）

PulseOn 的 LTTB 降采样通过自研的 DuckDB 扩展 [`duckdb-lttb`](https://github.com/kaikai/duckdb-lttb) 实现，而非在 Rust 端重新实现算法。这带来三个关键优势：

1. **数据不动**：降采样在 DuckDB 引擎内执行，直接扫描 DuckLake 管理的 Parquet 文件，无需将 100k+ 点通过 FFI 传到 Rust 端再传回。
2. **SQL 原生**：agent 查询路径（`get_metric_digest`）直接在 SQL 中调用 `lttb_sorted(step, value_f64, 50)`，与 DuckDB 的分区裁剪、列裁剪协同工作。
3. **性能验证**：benchmark 显示 duckdb-lttb 在所有场景下均快于 ClickHouse（1M sorted: 16ms vs 18ms，1M 100-group: 12ms vs 21ms），比 Python `lttb` 快 15x。

**duckdb-lttb 提供的函数**：

| 函数 | 说明 | 返回类型 |
|------|------|---------|
| `lttb(x, y, n)` | 标准 LTTB，内部按 x 排序后采样 | `STRUCT(x typed, y typed)[]` |
| `largestTriangleThreeBuckets(x, y, n)` | `lttb` 别名（ClickHouse 兼容） | 同上 |
| `lttb_sorted(x, y, n)` | 跳过排序的快速路径，调用方保证输入已按 x 排序 | 同上 |
| `lttb_indices(x, y, n)` | 返回选中点的排序后位置索引 | `BIGINT[]` |

**PulseOn 的 `get_metric_digest()` 使用 `lttb_sorted`**：

PulseOn 的 `metric_points` 表按 `(project_id, run_id, metric_name)` 分区，且每个分区内的数据按 `step` 天然有序。使用 `lttb_sorted` 可跳过内部排序（排序占打乱输入总时间的 78%），将 1M 点的降采样从 74ms 降至 16ms（4.6x 提升）。

```sql
-- get_metric_digest 的核心 SQL（Native 实现）
-- 使用 lttb_sorted 因为 metric_points 按 step 有序
SELECT lttb_sorted(step, value_f64, $max_points) AS digest
FROM dl.metric_points
WHERE run_id = $run_id AND metric_name = $metric_name
ORDER BY step;
```

**注意事项**：DuckDB 聚合函数不保证按 `ORDER BY` 前的顺序接收数据（可能并行执行聚合）。PulseOn 在 `get_metric_digest` 查询时设置 `SET threads = 1` 或使用 `preserve_insertion_order` 确保输入有序性，使 `lttb_sorted` 的前提成立。

**duckdb-lttb 相对于 ClickHouse / Python 的改进**：稳定排序（重复 x 保持插入顺序）、空桶保护、NULL 处理（validity mask 跳过）、排序快速路径（`lttb_sorted`）、索引输出（`lttb_indices`）、类型保持（DATE/TIMESTAMP/DECIMAL 输入输出保持类型）。

### 21.3 MinMaxLTTB（超大规模降采样，规划中）

对于超长训练（1M+ step），标准 LTTB 需要 O(n) 内存（1M points = 16MB/group）。MinMaxLTTB 通过两阶段策略将内存降至 O(buckets × 4)：

1. **MinMax 预选**：将数据分到 coarse buckets，每桶保留 first/last/min/max（4 点）
2. **LTTB 精选**：对预选候选集运行标准 LTTB

**规划 API**：`minmax_lttb(x, y, n, coarse_buckets)` — 参考 `plotly-resampler` 的 `MinMaxLTTB`。这是近似算法（非精确 LTTB），在 duckdb-lttb 扩展中实现（TODO P1）。

**PulseOn 映射**：当 `metric_points` 的某 run 超过 1M 个点时，`get_metric_digest(max_points=50)` 自动切换到 `minmax_lttb`。

### 21.4 Bucket 统计降采样（分布分析，规划中）

当 agent 需要理解数据分布而非视觉形状时，bucket 统计比 LTTB 更有价值：

**规划 API**：`bucket_stats(x, y, num_buckets) → STRUCT(bucket_start, bucket_end, count, min, max, mean, std, first, last)[]`

**PulseOn 映射**：在 `AgentToolInterface` 中作为 `get_metric_digest(algorithm='bucket_stats')` 的替代算法。Agent 可以先用 `bucket_stats` 理解分布，再用 `lttb` 看曲线形状。在 duckdb-lttb 扩展中实现（TODO P1）。

### 21.5 预计算策略

Run finish 时自动预计算 digest（使用 duckdb-lttb 扩展）：

```rust
impl PulseOnClient {
    async fn finish_run(&self, run_id: &str) -> Result<()> {
        // ... 现有 flush/compact 逻辑 ...

        // 预计算所有 metric 的 digest（三个常用点数）
        // 通过 DuckDB SQL 调用 lttb_sorted，结果写入 metric_digests 缓存表
        let metrics = self.catalog.list_metrics(run_id).await?;
        for metric in &metrics {
            for max_points in [30, 50, 100] {
                // SQL: SELECT lttb_sorted(step, value_f64, $max_points)
                //      FROM dl.metric_points WHERE run_id = ... AND metric_name = ...
                //      ORDER BY step
                // → 结果序列化为 JSON → INSERT INTO dl.metric_digests
                self.compute_and_cache_digest(run_id, &metric.name, max_points).await?;
            }
        }
        // 预计算 run summary markdown
        let _ = self.get_agent().summarize_run(run_id).await;
        Ok(())
    }
}
```

### 21.6 降采样性能基准（duckdb-lttb benchmark）

| 场景 | DuckDB lttb | ClickHouse | Python lttb |
|------|------------|-----------|-------------|
| 1M sorted, n=1000 | **16ms** | 18ms | 248ms |
| 1M shuffled, n=1000 | **74ms** | 85ms | N/A |
| 1M TIMESTAMP, n=1000 | **16ms** | 17ms | 255ms |
| 1M 100-group, n=100/group | **12ms** | 21ms | N/A |
| 10K sorted, n=100 | **1.0ms** | 2.0ms | 3.1ms |

> DuckDB 计时含 CLI 启动（~1ms），ClickHouse 为服务端纯查询时间。DuckDB 实际查询执行时间更低。
> PulseOn 使用 `lttb_sorted` 后，1M 点从 74ms（含排序）降至 16ms（跳过排序）。

---

## 22. AI Native 与现有架构的关系

### 22.1 AgentToolInterface vs QueryInterface

| 维度 | QueryInterface | AgentToolInterface |
|------|---------------|-------------------|
| **目标用户** | Developer, Trainer（人类） | AI Agent（LLM） |
| **返回类型** | Arrow RecordBatch | JSON / Markdown / 紧凑 struct |
| **数据粒度** | 原始（100K 点） | 降采样/摘要（50-200 点） |
| **语义层** | 无（原始 SQL 语义） | 有（find_best, compare, detect_anomaly） |
| **输出优化** | 列式 Arrow | Token 预算控制、紧凑序列 |
| **Tool schema** | 无 | 有（OpenAI/Anthropic/MCP） |
| **暴露方式** | Python SDK | Python SDK + MCP Server |

**关系**：`AgentToolInterface` **组合** `QueryInterface` 的能力（非继承）。Agent 工具方法内部先调用 `QueryInterface` 获取原始数据，再做降采样/统计/格式化后返回。

```rust
pub struct DuckDBAgentTools {
    query: Arc<dyn QueryInterface>,     // 复用现有查询能力
    catalog: Arc<dyn CatalogLayer>,     // 元数据读写
    conn: Arc<duckdb::Connection>,      // 直接 SQL（复杂查询）
    output_format: OutputFormat,
    token_budget: u32,
}
```

### 22.2 新增表与 DuckLake 的关系

所有新增表都是 **DuckLake catalog table**（无分区，行式存储），在 catalog.sqlite 的 DuckLake 内联数据中。不产生 Parquet 文件。如果未来数据量增长，可通过 `ALTER TABLE ... SET PARTITIONED BY` 转为 Parquet 分区表。

### 22.3 三种 Agent 接入方式

| 接入方式 | 适用场景 | 实现 |
|---------|---------|------|
| **Python SDK agent 模块** | 自定义 agent、Jupyter 分析、auto-research 脚本 | `client.agent.find_best_runs(...)` |
| **LLM function calling** | OpenAI/Anthropic/Gemini agent 通过 tool calling | `agent.get_tool_definitions(format="openai")` + `agent.execute_tool(...)` |
| **MCP Server** | Claude Code / Cursor / Codex 等 agent IDE 零集成 | `pulseon mcp serve --path ./runs` |

三种方式共享同一 `AgentToolInterface` 实现，只是暴露协议不同。

### 22.4 Cloud 版本的 Agent 实现差异

| 方法 | Native (DuckDB) | Cloud (ClickHouse + PG) |
|------|----------------|------------------------|
| `find_best_runs` | DuckDB SQL | ClickHouse SQL 或 PG run_summary |
| `detect_anomaly` | DuckDB window functions | ClickHouse `windowFunnel` |
| `get_metric_digest` | DuckDB `lttb_sorted()` 扩展 | ClickHouse `largestTriangleThreeBuckets()` |
| `get_experiment_lineage` | DuckDB recursive CTE | PG recursive CTE |
| MCP Server | stdio（本地 agent IDE） | HTTP/SSE（远程 agent） |

降采样在数据库引擎内执行（Native: duckdb-lttb 扩展，Cloud: ClickHouse 内置函数），避免数据跨 FFI 传输。输出格式化、token 预算控制在 Rust 端统一实现。

---

## 附录 C：AI Native 技术选型验证依据

| 组件 | 选型 | 验证状态 | 来源 |
|------|------|---------|------|
| MCP Server | 内置 MCP server（JSON-RPC over stdio） | ✅ W&B GA MCP server 验证模式，DuckDB MCP server 多实现存在 | wandb-mcp-server, duckdb-mcp-server |
| LTTB 降采样 | duckdb-lttb 扩展（`lttb_sorted` / `minmax_lttb` 规划中） | ✅ 自研 DuckDB 扩展，benchmark 超越 ClickHouse（1M sorted 16ms vs 18ms），99 条 SQLLogicTest 断言通过 | duckdb-lttb 项目, Sveinn Steinarsson 2013 |
| Tool schema 三格式 | OpenAI `parameters` / Anthropic `input_schema` / MCP `inputSchema` | ✅ 2026 主流 LLM 平台格式 | OpenAI/Anthropic/MCP spec |
| Auto-research 模式 | AIDE tree search + AutoResearch 线性迭代 | ✅ AIDE 4K+ stars, AutoResearch 病毒式传播 | WecoAI/aideml, karpathy/AutoResearch |
| 渐进式披露 | L1 摘要 → L2 关键点 → L3 降采样 → L4 原始 | ✅ 15+ 源共识 | LangChain/LlamaIndex/AutoGen 实践 |
| Agent IDE 集成 | MCP 配置指向 pulseon 二进制 | ✅ Claude Code/Cursor/Codex 原生支持 MCP | MCP spec 2025-06-18 |

---

## 附录 D：AI Native 数据模型快速对照表

| 实体 | 存储 | 分区 | 写入频率 | 查询模式 |
|------|------|------|---------|---------|
| `hypotheses` | catalog | 无 | 低（每实验 1 次） | `WHERE project_id + status` |
| `experiment_lineage` | catalog | 无 | 低（每 run 1 次） | Recursive CTE |
| `agent_decisions` | catalog | 无 | 中（每 run 5-50 次） | `WHERE run_id + decision_type` |
| `insights` | catalog | 无 | 低（每 run 2-10 次） | `WHERE project_id/run_id` |
| `reports` | catalog | 无 | 极低（每 project 1-5 次） | `WHERE project_id` |
| `metric_digests` | catalog | 无 | 低（run finish 预计算 + 按需） | `WHERE run_id + metric_name + max_points` |

---

**文档版本**: v2.1（Part I 基础架构 + Part II AI Native 扩展 + duckdb-lttb 集成）
**参考文档**: `docs/reference/training-metrics-hosting-boundaries.md` · `docs/reference/training-metrics-storage-architecture.md` · `docs/reference/ducklake-archive.md`
**AI Native 参考**: W&B MCP Server (wandb-mcp-server) · AIDE (WecoAI/aideml) · AutoResearch (karpathy/AutoResearch) · MCP Protocol (modelcontextprotocol.io) · LTTB (Sveinn Steinarsson 2013)
**降采样实现**: duckdb-lttb 扩展（`lttb_sorted` / `lttb_indices` / `minmax_lttb` 规划中 / `bucket_stats` 规划中）
