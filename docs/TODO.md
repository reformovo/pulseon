# PulseOn 实现路线图

> 基于 `docs/native-architecture.md` v2.1 提取的实现工作项。
> 当前状态：Phase 0 完成（脚手架搭建，31 个 Rust 源文件，13 个依赖，cargo check + maturin develop + pytest 全部通过）。

---

## Phase 0：项目脚手架 ✅

> 依赖：无
> 架构参考：§6 Rust Crate/模块结构, §6.3 Cargo.toml
> 实现文档：`docs/impl-phase-0-scaffold.md`
> Commit：`c4b473d`

- [x] 更新 `Cargo.toml`：添加全部依赖（最新版本，经 @oracle 审查）
  - `pyo3 = { version = "0.29", features = ["extension-module"] }`
  - `pyo3-arrow = "0.19"`
  - `duckdb = { version = "~1.10504.0", features = ["bundled", "loadable-extension"] }`
  - `tokio = { version = "1.52", features = ["full"] }`
  - `async-trait = "0.1"`
  - `thiserror = "2"`（`anyhow` 延迟到 Phase 4）
  - `serde = { version = "1.0", features = ["derive"] }`, `serde_json = "1.0"`
  - `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`
  - `uuid = { version = "1.0", features = ["v4"] }`
  - `chrono = { version = "0.4", features = ["serde"] }`
  - `[features] default = ["native"]`
  - `[profile.release] strip = true`
  - 注：`arrow` dep 未添加（duckdb-rs 内部 re-export，避免版本冲突）
- [x] 创建模块目录结构（§6.1，31 个 .rs 文件）
  - `src/model/` — `mod.rs` + 8 leaf（types, run, metric, event, summary, config, tag, artifact）
  - `src/catalog/` — `mod.rs`（含 CatalogError）+ 4 leaf（trait_def, ducklake_impl, cloud_impl, types）
  - `src/storage/` — `mod.rs` + 4 leaf（trait_def, local, s3, ducklake_bridge）
  - `src/compute/` — `mod.rs` + 4 leaf（trait_def, duckdb_impl, cloud_impl, query_interface）
  - `src/engine/` — `mod.rs`（含 EngineError）+ 3 leaf（client, write, flush）
  - `src/sdk/` — `mod.rs`（含 SdkError）+ 4 leaf（client, run, config, query）
  - 注：`trait_def.rs` 而非 `trait.rs`（Rust 保留关键字）；`error.rs` 合并入 `mod.rs`（@oracle simplify 建议）
- [x] 替换 `src/lib.rs`：移除 `sum_as_string`，改为模块声明 + 空 `#[pymodule]`
- [x] 删除 `tests/test_sum.py`，创建 `tests/test_init.py`（验证 `import pulseon`）
- [x] 验证 `cargo check`（0 warnings）+ `maturin develop` + `pytest`（1 passed）

**验收**：✅ `cargo check` 0 warnings, `maturin develop` 构建成功, `pytest` 1 passed, 无 placeholder 代码残留。

---

## Phase 1：数据模型（model/）

> 依赖：Phase 0
> 架构参考：§3 逻辑数据模型, §9.1 配置结构

- [ ] `model/types.rs`：newtype ID 类型
  - `WorkspaceId(String)`, `ProjectId(String)`, `RunId(String)`, `MetricName(String)`
  - `Display` / `AsRef<str>` trait impl
- [ ] `model/run.rs`：`Run`, `RunStatus`（running/finished/failed/crashed）, `RunFilter`, `RunSort`
- [ ] `model/metric.rs`：`MetricDefinition`, `MetricPoint`, `ValueType`（f64/i64/str/bool）
- [ ] `model/event.rs`：`RunEvent`, `RunEventType`
- [ ] `model/summary.rs`：`RunSummary`, `RunSummaryRow`, `SummaryFilter`, `SummarySort`
- [ ] `model/config.rs`：`PulseOnConfig`, `DeploymentMode`, `CatalogConfig`, `DuckDBConfig`, `FlushConfig`
  - 所有 struct derive `Serialize, Deserialize`
- [ ] `model/tag.rs`：`Tag`
- [ ] `model/artifact.rs`：`Artifact`
- [ ] `model/mod.rs`：重新导出所有公共类型
- [ ] `engine/error.rs`：统一错误类型 `PulseOnError`（thiserror），涵盖 catalog/storage/compute/sdk 错误

**验收**：`cargo check` 通过，所有类型可被其他模块引用，无 I/O 依赖。

---

## Phase 2：Catalog 层

> 依赖：Phase 1
> 架构参考：§2.2 CatalogLayer trait, §3.3 核心表 Schema, §9.4 初始化流程

- [ ] `catalog/trait.rs`：定义 `CatalogLayer` async trait（§2.2 全部方法）
  - Workspace / Project / Run CRUD
  - MetricDefinition register/list
  - RunSummary upsert/get/query
  - Tag / Config / Artifact
  - `initialize()` / `shutdown()`
- [ ] `catalog/types.rs`：`FileInfo`, `SnapshotInfo` 等 catalog 层特有类型
- [ ] `catalog/ducklake_impl.rs`：`DuckLakeSqliteCatalog` 实现
  - 持有 `Arc<duckdb::Connection>`
  - `initialize()`：`INSTALL ducklake; LOAD ducklake; ATTACH ... (TYPE ducklake, CATALOG 'sqlite', DATA_PATH ..., DATA_INLINING_ROW_LIMIT 500)`
  - `initialize()`：创建业务表（`CREATE TABLE IF NOT EXISTS dl.runs/metric_definitions/metric_points/run_events/run_summary/tags/run_tags/configs/artifacts`）
  - `initialize()`：设置分区 `ALTER TABLE dl.metric_points SET PARTITIONED BY (project_id, run_id, metric_name)`
  - `create_run()` / `register_metric()` 等：通过 DuckDB SQL `INSERT INTO dl.<table>`
  - `list_runs()` / `query_run_summaries()`：通过 DuckDB SQL `SELECT ... FROM dl.<table>`
  - `finish_run()`：更新 status + 计算 summary + upsert
  - `shutdown()`：`CHECKPOINT dl` + `DETACH dl`
- [ ] `catalog/cloud_impl.rs`：`PostgresCatalog` skeleton（`unimplemented!()` 占位）

**验收**：Rust 单元测试可创建 catalog、create_run、log_metric（通过 SQL INSERT）、list_runs、finish_run（触发 flush），数据持久化到 SQLite + Parquet。

---

## Phase 3：Compute 层 + 查询接口

> 依赖：Phase 2
> 架构参考：§2.4 ComputeLayer trait, §5 查询路径, §5.2 QueryInterface, §5.3 pyo3-arrow

- [ ] `compute/trait.rs`：定义 `ComputeLayer` async trait（§2.4）
- [ ] `compute/query_interface.rs`：定义 `QueryInterface` async trait（§5.2）
  - `list_runs`, `list_metrics`, `query_metric_series`, `query_run_summaries`, `list_run_events`, `execute_raw`
- [ ] `compute/duckdb_impl.rs`：`DuckDBCompute` 实现 `ComputeLayer`
  - 持有 `Arc<duckdb::Connection>`（与 catalog 共享）
  - `execute()`：执行 SQL，通过 `query_arrow()` 返回 `Vec<RecordBatch>`
- [ ] `compute/duckdb_impl.rs`：`DuckDBQueryInterface` 实现 `QueryInterface`
  - `query_metric_series()`：生成 `SELECT ... FROM dl.metric_points WHERE run_id IN (...) ORDER BY step`
  - 其他方法：SQL 生成 + Arrow 返回
- [ ] `compute/cloud_impl.rs`：`ClickHouseCompute` + `ClickHouseQueryInterface` skeleton

**验收**：Rust 单元测试可查询已写入的 metric_points，返回 Arrow RecordBatch，验证分区裁剪生效（查询特定 run_id 只扫描对应目录）。

---

## Phase 4：Engine 编排层 + 写入路径

> 依赖：Phase 2, Phase 3
> 架构参考：§4 写入路径, §4.3 Flush 触发规则, §9.4 初始化流程

- [ ] `engine/client.rs`：`PulseOnClient` struct
  - 持有 `Arc<dyn CatalogLayer>`, `Arc<dyn QueryInterface>`, `Arc<duckdb::Connection>`
  - `new()`：接收 config，创建 DuckDB connection，加载扩展（ducklake + lttb），ATTACH catalog，初始化表
  - `get_agent()` accessor（延迟创建，Phase 7 实现）
- [ ] `engine/write.rs`：写入路径
  - `log_metric(run_id, name, step, value)`：验证 metric_definition（首次自动注册）→ `INSERT INTO dl.metric_points`
  - `log_metrics(run_id, metrics, step)`：批量 INSERT（多 VALUES）
  - `log_event(run_id, event_type, message)`：`INSERT INTO dl.run_events`
  - `create_run(project, name, config, tags)`：`INSERT INTO dl.runs` + 可选 tags/config
- [ ] `engine/flush.rs`：flush 编排
  - `finish_run(run_id)`：`catalog.finish_run()` → `CALL ducklake_flush_inlined_data('dl')` → 可选 `ducklake_merge_adjacent_files`
  - 定期 auto flush 定时器（`auto_flush_interval_secs` 配置）

**验收**：Rust 集成测试：create_run → log_metric × 1000 → finish_run，验证 Parquet 文件生成、数据可查询、内联数据已 flush。

---

## Phase 5：Python SDK（Developer-facing）

> 依赖：Phase 4
> 架构参考：§7 职责边界, §8 Python SDK API, §9.2 Python 配置

- [ ] `sdk/config.rs`：`PyConfig` — Python dict → Rust `PulseOnConfig` 转换
- [ ] `sdk/client.rs`：`#[pyclass] Client`
  - `#[new] fn new(config)` / `fn init(path, workspace, ...) -> Client`
  - `create_run(project, name, config, tags) -> Run`
  - `list_runs(project) -> Vec<PyObject>`
  - `query_metric_series(run_ids, metrics, step_range) -> PyRecordBatchReader`
  - `agent` property（延迟创建，Phase 7 实现）
- [ ] `sdk/run.rs`：`#[pyclass] Run`
  - `log_metric(name, step, value, phase)`
  - `log_metrics(metrics, step, phase)`
  - `log_event(event_type, message, metadata)`
  - `finish()`
  - `__enter__` / `__exit__`（上下文管理器）
- [ ] `sdk/query.rs`：查询结果返回
  - `query_metric_series` 返回 `PyRecordBatchReader`（pyo3-arrow 零拷贝）
- [ ] `sdk/error.rs`：`PulseOnError` → `PyErr` 转换
- [ ] `lib.rs`：`#[pymodule] fn _pulseon` 注册 Client, Run, init 函数
- [ ] `python/pulseon/__init__.py`：Python 侧薄封装
  - `init(path, workspace, **kwargs) -> Client`
  - 重新导出 Client, Run
- [ ] `python/pulseon/_pulseon.pyi`：type stub 更新

**验收**：`pulseon.init("./test_runs")` → `create_run` → `log_metric` × 100 → `finish` → `query_metric_series` 返回 pyarrow Table，端到端 Python 测试通过。

---

## Phase 6：AI Native 数据模型

> 依赖：Phase 2
> 架构参考：§12 数据模型扩展, §20.2 CatalogLayer 新增方法

- [ ] `model/hypothesis.rs`：`Hypothesis`, `HypothesisStatus`, `NewHypothesis`
- [ ] `model/lineage.rs`：`ExperimentLineage`, `LineageDirection`, `NewLineage`
- [ ] `model/agent_decision.rs`：`AgentDecision`, `DecisionType`, `NewAgentDecision`
- [ ] `model/insight.rs`：`Insight`, `InsightType`, `InsightTarget`, `NewInsight`
- [ ] `model/report.rs`：`Report`, `NewReport`
- [ ] `model/digest.rs`：`MetricDigest`, `MetricPointCompact`, `BucketStats`
- [ ] `model/agent_types.rs`：`RunRanking`, `RunComparison`, `ComparisonRow`, `MetricStats`, `AnomalyReport`, `ExperimentTree`, `RunSummaryMarkdown`, `TrendDirection`, `AnomalyType`, `AnomalySeverity`
- [ ] `catalog/trait.rs`：新增 9 组 AI Native CRUD 方法（§20.2）
  - Hypothesis: create/get/list/update_status
  - Lineage: add/get（递归 CTE）
  - AgentDecision: log/get
  - Insight: add/get/update
  - Report: create/get
  - MetricDigest: get_cached/cache
- [ ] `catalog/ducklake_impl.rs`：实现新增方法
  - `initialize()` 扩展：创建 6 张新表（hypotheses, experiment_lineage, agent_decisions, insights, reports, metric_digests）
  - `runs` 表 `ALTER TABLE ADD COLUMN hypothesis_id, parent_run_id`
  - 各 CRUD 方法的 SQL 实现

**验收**：Rust 测试可 create_hypothesis → create_run(hypothesis_id) → add_experiment_lineage → log_agent_decision → add_insight → create_report，数据持久化到 catalog。

---

## Phase 7：AgentToolInterface（语义查询层）

> 依赖：Phase 6, Phase 3
> 架构参考：§13 语义查询层, §13.3 DuckDB SQL 映射

- [ ] `compute/agent_tools.rs`：定义 `AgentToolInterface` async trait（§13.1 全部方法）
  - find_best_runs, search_runs, compare_runs, detect_anomaly, detect_project_anomalies
  - summarize_run, summarize_project
  - get_metric_digest, get_metric_digests_batch
  - get_experiment_lineage, list_hypotheses, get_insights, get_agent_decisions
  - recommend_next_experiment, describe_schema, list_tools
- [ ] `compute/semantic.rs`：语义 SQL builder
  - `find_best_runs_sql()`：§13.3 的 COALESCE 查询
  - `compare_runs_sql()`：GROUP BY run_id, metric_name 聚合
  - `detect_anomaly_sql()`：窗口函数（NaN/发散/plateau/spike 检测）
  - `lineage_cte_sql()`：递归 CTE
  - `search_runs_sql()`：名称/tag/config 过滤
- [ ] `compute/agent_duckdb_impl.rs`：`DuckDBAgentTools` 实现 `AgentToolInterface`
  - 持有 `Arc<dyn QueryInterface>`, `Arc<dyn CatalogLayer>`, `Arc<duckdb::Connection>`
  - `get_metric_digest()`：先查 `metric_digests` 缓存 → 无缓存则 SQL `lttb_sorted()` → 序列化 → 写缓存
  - `summarize_run()`：聚合 run 元数据 + metric stats → 生成 markdown
  - `detect_anomaly()`：执行窗口函数 SQL → 解析结果为 `AnomalyReport`
- [ ] `compute/agent_cloud_impl.rs`：`ClickHouseAgentTools` skeleton
- [ ] `compute/output.rs`：`OutputFormat` 枚举, token 估算, `AgentResponse<T>`, `AgentResponseMeta`

**验收**：Rust 测试：写入 10K 点 → `find_best_runs` 返回正确排名 → `compare_runs` 返回统计表 → `detect_anomaly` 检测注入的 NaN → `get_metric_digest(50)` 返回 50 点降采样序列。

---

## Phase 8：LLM 输出格式 + Tool-Calling

> 依赖：Phase 7
> 架构参考：§14 输出格式, §15 Tool-Calling 接口

- [ ] `compute/output.rs` 扩展：
  - `to_compact_json()`：字段名缩短、数值截断
  - `to_markdown_table()`：表格化输出
  - `estimate_tokens()`：`len_bytes / 3` 启发式
  - Token 预算控制：超预算时自动减少 top_k / max_points
- [ ] `sdk/tools.rs`：Tool schema 导出
  - `ToolDefinition`, `ToolParameters`, `ParameterProperty`, `ToolReturnDescription` struct
  - `to_openai_tools()`：`{"type": "function", "function": {"name", "description", "parameters"}}`
  - `to_anthropic_tools()`：`{"name", "description", "input_schema"}`
  - `to_mcp_tools()`：`{"name", "description", "inputSchema"}`
  - 10 个默认 tool 定义（find_best_runs, search_runs, compare_runs, detect_anomaly, summarize_run, summarize_project, get_metric_digest, get_experiment_lineage, list_hypotheses, get_insights）
- [ ] `engine/agent.rs`：`execute_tool(agent, tool_name, arguments, output_format) -> AgentToolResult`
  - match tool_name → 调用对应 AgentToolInterface 方法 → 格式化结果
- [ ] `sdk/agent.rs`：`#[pyclass] Agent`
  - `get_tool_definitions(format) -> list[dict]`
  - `execute_tool(tool_name, arguments) -> AgentToolResult`
  - 所有语义查询方法的 Python 暴露
  - 写入方法：`add_insight`, `log_agent_decision`, `generate_report`
- [ ] `python/pulseon/agent/__init__.py`：Python `Agent` 类薄封装
  - `client.agent` property 延迟创建

**验收**：Python 测试：`agent.get_tool_definitions("openai")` 返回合法 JSON schema → `agent.execute_tool("find_best_runs", {...})` 返回 JSON 结果 → 可传入 OpenAI `chat.completions.create(tools=...)` 完成 tool-calling 循环。

---

## Phase 9：MCP Server

> 依赖：Phase 8
> 架构参考：§16 MCP Server

- [ ] `mcp/mod.rs`：MCP server 模块入口
- [ ] `mcp/server.rs`：`PulseOnMcpServer`
  - `serve_stdio()`：JSON-RPC over stdio
  - 请求循环：`tools/list` → 返回 tool 定义（`inputSchema` camelCase）, `tools/call` → 分发到 `execute_tool()`, `resources/read` → 返回 resource 数据
  - 错误处理：`method_not_found` 等 JSON-RPC 错误码
- [ ] `mcp/tools.rs`：MCP tool schema 格式导出（`inputSchema` camelCase）
- [ ] `mcp/resources.rs`：MCP resources
  - `pulseon://projects` → project 列表 JSON
  - `pulseon://project/{id}/summary` → markdown
  - `pulseon://run/{id}/summary` → markdown
  - `pulseon://run/{id}/metrics/digest` → JSON
  - `pulseon://project/{id}/insights` → JSON
  - `pulseon://project/{id}/hypotheses` → JSON
  - `pulseon://project/{id}/reports/{rid}` → markdown
- [ ] CLI 入口：`pulseon mcp serve --path ./runs`（或通过 Python `pulseon.mcp.serve()`)
- [ ] `python/pulseon/mcp/__init__.py`：`serve(path, transport)`, `get_config(path)`

**验收**：配置 Claude Code MCP（`.claude/mcp.json` 指向 `pulseon mcp serve`）→ Claude Code 可调用 `find_best_runs` 等 tool → 返回实验数据。

---

## Phase 10：降采样集成

> 依赖：Phase 7
> 架构参考：§21 降采样策略, §9.4 扩展加载

- [ ] `engine/client.rs`：初始化时加载 lttb 扩展
  - `INSTALL lttb; LOAD lttb;`（在 ducklake 加载之后）
- [ ] `compute/agent_duckdb_impl.rs`：`get_metric_digest()` 实现
  - SQL: `SELECT lttb_sorted(step, value_f64, $max_points) FROM dl.metric_points WHERE run_id = ... AND metric_name = ... ORDER BY step`
  - `SET threads = 1` 确保输入有序性（`lttb_sorted` 前提）
  - 结果解析为 `MetricDigest` struct（series + stats）
  - 缓存写入 `dl.metric_digests` 表
- [ ] `engine/flush.rs`：`finish_run()` 预计算 digest
  - 遍历所有 metric × [30, 50, 100] 点数 → `compute_and_cache_digest()`
  - 预计算 `summarize_run()` markdown
- [ ] `compute/agent_duckdb_impl.rs`：`get_metric_digests_batch()` 批量查询
- [ ] duckdb-lttb 扩展依赖：确认 `INSTALL lttb` 在目标平台可用（macOS ARM64 / Linux x86_64），离线场景预下载 `.duckdb_extension`

**验收**：写入 100K 点 → `finish_run` → `get_metric_digest(50)` 返回 50 点 → 验证 digest 已缓存（第二次查询从 `metric_digests` 表读取）→ 验证 `lttb_sorted` 比 `lttb` 快（排序开销消除）。

---

## Phase 11：实时监控 + Auto-Research

> 依赖：Phase 8, Phase 10
> 架构参考：§17 Auto-Research 工作流, §18 实时监控

- [ ] `compute/agent_tools.rs` 扩展：`AgentMonitor` trait
  - `get_run_status(run_id) -> RunStatusSnapshot`
  - `get_recent_metrics(run_id, since_step) -> RecentMetricsSnapshot`
  - `get_live_summary(run_id) -> LiveSummary`
  - `watch_condition(run_id, condition, callback) -> watcher_id`
  - `unwatch(watcher_id)`
- [ ] `engine/agent.rs`：`WatchManager`
  - `watch_condition` 实现：`tokio::spawn` 后台轮询任务（30s 间隔）
  - 条件检测：no_improvement / value_above / value_below / nan_detected
  - 回调触发：`AlertEvent` 传入 callback
- [ ] `sdk/agent.rs` 扩展：Python 暴露监控方法
  - `watch_condition(run_id, condition, callback) -> str`
  - `unwatch(watcher_id)`
  - `get_run_status`, `get_recent_metrics`, `get_live_summary`
- [ ] `python/pulseon/agent/auto.py`：`AutoResearchAgent` 高级封装
  - `run_auto_research_loop(project_id, objective, max_iterations)`
  - 6 步循环：gather_context → propose_experiment → launch_experiment → monitor → analyze → generate_insights
  - LLM 调用：通过 `openai` / `anthropic` Python SDK
  - 实验谱系：`add_experiment_lineage` 记录 run 间关系
  - 假设管理：`create_hypothesis` → `update_hypothesis_status`

**验收**：Python 集成测试：启动 mock 训练 → `watch_condition` 检测到 plateau → 回调触发 → `AutoResearchAgent` 完成 1 轮 auto-research 循环（假设 → 实验 → 分析 → 洞察）。

---

## Phase 12：测试、CI、文档

> 依赖：所有 Phase
> 架构参考：全文

- [ ] Rust 单元测试
  - `model/` 类型测试
  - `catalog/` DuckLake CRUD 测试（临时目录）
  - `compute/` 查询测试（预填充数据）
  - `engine/` 写入/flush/finish 端到端测试
- [ ] Rust 集成测试（`tests/`）
  - `test_init.rs`：init → create_run → log_metric → finish → query 端到端
  - `test_agent.rs`：agent tools 端到端
  - `test_mcp.rs`：MCP server stdio 交互测试
- [ ] Python 测试（`tests/`）
  - `test_basic.py`：init, create_run, log_metric, finish, query
  - `test_agent.py`：agent.find_best_runs, summarize_run, get_metric_digest
  - `test_tool_calling.py`：get_tool_definitions + execute_tool 循环
  - `test_auto_research.py`：AutoResearchAgent mock LLM 测试
- [ ] CI（`.github/workflows/`）
  - `cargo test` + `maturin develop && pytest`
  - macOS ARM64 + Linux x86_64 矩阵
  - `cargo clippy` + `cargo fmt --check`
- [ ] 文档
  - `README.md`：快速开始、安装、基本用法
  - `docs/README.zh.md`：中文 README
  - API reference（从 docstring 生成）
- [ ] S3 模式测试
  - MinIO 容器 → `pulseon.init("s3://...")` → 写入/查询验证

**验收**：CI 全绿，`pytest` 全部通过，README 有可运行的快速开始示例。

---

## 依赖关系图

```
Phase 0 (scaffold)
  │
  ├─► Phase 1 (model)
  │     │
  │     ├─► Phase 2 (catalog)
  │     │     │
  │     │     ├─► Phase 4 (engine + write)
  │     │     │     │
  │     │     │     ├─► Phase 5 (Python SDK)
  │     │     │     │
  │     │     │     └─► Phase 10 (downsampling)
  │     │     │
  │     │     ├─► Phase 6 (AI Native model)
  │     │     │     │
  │     │     │     └─► Phase 7 (AgentToolInterface)
  │     │     │           │
  │     │     │           ├─► Phase 8 (LLM output + tools)
  │     │     │           │     │
  │     │     │           │     ├─► Phase 9 (MCP server)
  │     │     │           │     │
  │     │     │           │     └─► Phase 11 (auto-research)
  │     │     │           │
  │     │     │           └─► Phase 10 (downsampling)
  │     │     │
  │     ├─► Phase 3 (compute + query)
  │
  └─► Phase 12 (tests + CI) ← 依赖所有 Phase
```

**可并行**：Phase 2 和 Phase 3 可并行（都依赖 Phase 1）。Phase 6 可在 Phase 4 之前开始（只依赖 Phase 2）。Phase 10 可在 Phase 8 之前开始（只依赖 Phase 7）。

---

## 优先级建议

| 优先级 | Phase | 理由 |
|--------|-------|------|
| **P0** | 0, 1, 2, 3, 4, 5 | 基础架构 + Developer SDK，端到端可用 |
| **P1** | 6, 7, 10 | AI Native 数据模型 + 语义查询 + 降采样，agent 核心能力 |
| **P1** | 8 | Tool-calling，LLM agent 可接入 |
| **P2** | 9 | MCP server，agent IDE 零集成 |
| **P2** | 11 | Auto-research 高级封装 |
| **P3** | 12 | 测试 CI 文档（贯穿全程，每个 Phase 完成时同步补充） |

**MVP 里程碑**：Phase 0-5 完成 = Developer 可用（`pulseon.init` → `create_run` → `log_metric` → `query`）。
**AI Native 里程碑**：Phase 6-8 + 10 完成 = Agent 可用（`client.agent.find_best_runs` → `get_metric_digest` → tool-calling 循环）。
**Auto Research 里程碑**：Phase 9 + 11 完成 = Auto-research 可用（MCP server + `AutoResearchAgent` 循环）。
