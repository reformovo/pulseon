# Phase 0：项目脚手架实现文档

> Phase: 0
> 日期：2026-06-22
> Commit：`c4b473d`
> 架构参考：`docs/native-architecture.md` §6（Rust Crate/模块结构）, §6.3（Cargo.toml）
> 状态：✅ 完成

---

## 1. 目标

将 PulseOn 从 maturin/pyo3 placeholder 脚手架升级为符合架构设计的模块化项目结构，所有依赖使用最新版本。终态：`cargo check` 零警告，`maturin develop` 构建成功，`pytest` 通过，无 placeholder 代码残留。

## 2. 前置状态

- `Cargo.toml`：仅 `pyo3 = { version = "0.28.3", features = ["experimental-inspect"] }`
- `src/lib.rs`：12 行，placeholder `sum_as_string` + `#[pymodule] fn _pulseon`
- `python/pulseon/__init__.py`：`from ._pulseon import sum_as_string`
- `tests/test_sum.py`：测试 `sum_as_string(5, 20) == "25"`

## 3. 依赖版本（@librarian 验证，crates.io 2026-06-22）

| Crate | 版本 | Cargo.toml 行 | 说明 |
|-------|------|--------------|------|
| pyo3 | 0.29.0 | `pyo3 = { version = "0.29", features = ["extension-module"] }` | 0.28→0.29 breaking: 删除 `experimental-inspect` feature, 删除 0.27 deprecations |
| pyo3-arrow | 0.19.0 | `pyo3-arrow = "0.19"` | 依赖 pyo3 ^0.29 + arrow ^59，兼容 |
| duckdb | 1.10504.0 | `duckdb = { version = "~1.10504.0", features = ["bundled", "loadable-extension"] }` | 捆绑 DuckDB v1.5.4；`loadable-extension` 启用运行时 `LOAD ducklake; LOAD lttb;` |
| arrow | — | 未添加 | duckdb-rs 内部 re-export，避免版本冲突（Phase 3 按需添加） |
| tokio | 1.52.3 | `tokio = { version = "1.52", features = ["full"] }` | |
| async-trait | 0.1.89 | `async-trait = "0.1"` | |
| thiserror | 2.0.18 | `thiserror = "2"` | |
| anyhow | — | 未添加（延迟到 Phase 4） | @oracle 建议：scaffold 阶段无使用场景 |
| serde | 1.0.228 | `serde = { version = "1.0", features = ["derive"] }` | |
| serde_json | 1.0.150 | `serde_json = "1.0"` | |
| tracing | 0.1.44 | `tracing = "0.1"` | |
| tracing-subscriber | 0.3.23 | `tracing-subscriber = { version = "0.3", features = ["env-filter"] }` | |
| uuid | 1.23.3 | `uuid = { version = "1.0", features = ["v4"] }` | |
| chrono | 0.4.45 | `chrono = { version = "0.4", features = ["serde"] }` | |

**关键兼容性**：pyo3 0.29 + pyo3-arrow 0.19 + arrow 59 三者互相兼容（pyo3-arrow 0.19 依赖 pyo3 ^0.29 和 arrow ^59）。

**`loadable-extension` feature**：DuckDB 的扩展加载（`LOAD` SQL 命令）是编译时标志。不加此 feature，`LOAD ducklake` 会返回 "Extension loading is not enabled"。架构文档 §6.3 未列举此 feature，但它是运行时扩展加载的前提。

## 4. 模块结构（31 个 .rs 文件）

```
src/
├── lib.rs                        # 模块声明 + 空 #[pymodule]
│
├── model/                        # 逻辑数据模型（纯类型，无 I/O）
│   ├── mod.rs                    # pub mod 声明
│   ├── types.rs                  # TODO: Phase 1 — newtype IDs
│   ├── run.rs                    # TODO: Phase 1 — Run, RunStatus
│   ├── metric.rs                 # TODO: Phase 1 — MetricDefinition, MetricPoint
│   ├── event.rs                  # TODO: Phase 1 — RunEvent
│   ├── summary.rs                # TODO: Phase 1 — RunSummary
│   ├── config.rs                 # TODO: Phase 1 — PulseOnConfig
│   ├── tag.rs                    # TODO: Phase 1 — Tag
│   └── artifact.rs               # TODO: Phase 1 — Artifact
│
├── catalog/                      # CatalogLayer trait + 实现
│   ├── mod.rs                    # pub mod + CatalogError enum
│   ├── trait_def.rs              # TODO: Phase 2 — CatalogLayer trait
│   ├── ducklake_impl.rs          # TODO: Phase 2 — DuckLakeSqliteCatalog
│   ├── cloud_impl.rs             # TODO: future — PostgresCatalog
│   └── types.rs                  # TODO: Phase 2 — FileInfo, SnapshotInfo
│
├── storage/                      # StorageLayer trait + 实现
│   ├── mod.rs                    # pub mod 声明
│   ├── trait_def.rs              # TODO: Phase 2 — StorageLayer trait
│   ├── local.rs                  # TODO: Phase 2 — LocalStorage
│   ├── s3.rs                     # TODO: future — S3ObjectStorage
│   └── ducklake_bridge.rs        # TODO: Phase 2 — DuckLakeStorage
│
├── compute/                      # ComputeLayer + QueryInterface + 实现
│   ├── mod.rs                    # pub mod 声明
│   ├── trait_def.rs              # TODO: Phase 3 — ComputeLayer trait
│   ├── duckdb_impl.rs            # TODO: Phase 3 — DuckDBCompute
│   ├── cloud_impl.rs             # TODO: future — ClickHouseCompute
│   └── query_interface.rs        # TODO: Phase 3 — QueryInterface trait
│
├── engine/                       # 编排层
│   ├── mod.rs                    # pub mod + EngineError enum
│   ├── client.rs                 # TODO: Phase 4 — PulseOnClient
│   ├── write.rs                  # TODO: Phase 4 — 写入路径
│   └── flush.rs                  # TODO: Phase 4 — flush 编排
│
└── sdk/                          # PyO3 绑定层
    ├── mod.rs                    # pub mod + SdkError enum
    ├── client.rs                 # TODO: Phase 5 — #[pyclass] Client
    ├── run.rs                    # TODO: Phase 5 — #[pyclass] Run
    ├── config.rs                 # TODO: Phase 5 — PyConfig
    └── query.rs                  # TODO: Phase 5 — 查询结果返回
```

## 5. @oracle 审查决策

### 计划审查（批准，3 项修改）

| 决策 | 理由 |
|------|------|
| 移除 `arrow = "59"` 依赖 | duckdb-rs 内部 re-export arrow，显式添加可能导致版本冲突。Phase 3 实现查询接口时按需添加 |
| `mod.rs` 填充 `pub mod` 声明 | 空 `mod.rs` 会导致子模块成为不可达死代码 |
| `error.rs` 合并入 `mod.rs` | 10 行 error enum 不需要独立文件（simplify 建议） |

### Phase 审查（批准，5 项修改）

| 决策 | 理由 |
|------|------|
| 移除 `anyhow` 依赖 | scaffold 阶段无使用场景，Phase 4 按需添加 |
| `#[allow(dead_code)]` on 3 个 error enums | 消除编译警告，CI 可用 `-D warnings` |
| `EngineError` 添加双路径注释 | 说明 `duckdb::Error` 可通过 CatalogError 或直接到达 EngineError |
| `__all__: list[str] = []` → `__all__ = []` | Python 惯例不在模块级标注 `__all__` 类型 |
| `trait.rs` → `trait_def.rs` | `trait` 是 Rust 保留关键字，不能作为文件名 |

## 6. 错误类型层次

```
duckDB Error ─┬─► CatalogError ──► EngineError ──► SdkError ──► Python Exception
              │                    (also wraps        (wraps
              │                     duckdb::Error      EngineError
              │                     directly)          + PyErr)
              │
              └─► (duckdb::Error 可通过任一路径流入)
```

- `CatalogError`（`catalog/mod.rs`）：catalog 层错误，包装 `duckdb::Error`
- `EngineError`（`engine/mod.rs`）：编排层错误，包装 `CatalogError` + `duckdb::Error`
- `SdkError`（`sdk/mod.rs`）：PyO3 边界错误，包装 `EngineError` + `pyo3::PyErr`

## 7. 验证结果

| 检查 | 结果 |
|------|------|
| `cargo check` | ✅ 0 warnings, 0 errors |
| `uv run maturin develop` | ✅ CPython 3.13 wheel 构建成功 |
| `python -c "import pulseon"` | ✅ import OK |
| `pytest tests/test_init.py` | ✅ 1 passed |

## 8. 文件变更清单

| 文件 | 操作 |
|------|------|
| `Cargo.toml` | 修改：13 个依赖，最新版本 |
| `Cargo.lock` | 自动更新 |
| `src/lib.rs` | 替换：模块声明 + 空 `#[pymodule]` |
| `src/model/` | 新建：9 文件（mod.rs + 8 leaf） |
| `src/catalog/` | 新建：5 文件（mod.rs + 4 leaf） |
| `src/storage/` | 新建：5 文件（mod.rs + 4 leaf） |
| `src/compute/` | 新建：5 文件（mod.rs + 4 leaf） |
| `src/engine/` | 新建：4 文件（mod.rs + 3 leaf） |
| `src/sdk/` | 新建：5 文件（mod.rs + 4 leaf） |
| `python/pulseon/__init__.py` | 修改：移除 `sum_as_string` |
| `python/pulseon/_pulseon.pyi` | 修改：移除 `sum_as_string` stub |
| `tests/test_sum.py` | 删除 |
| `tests/test_init.py` | 新建：import 验证测试 |
| `.gitignore` | 修改：添加 `.slim/deepwork/` |
| `.ignore` | 新建：`!.slim/deepwork/` 例外 |

## 9. 下一步

Phase 1：数据模型（`model/`）— 实现 newtype ID、Run、MetricDefinition、MetricPoint、RunEvent、RunSummary、PulseOnConfig 等全部类型。依赖 Phase 0，无 I/O 依赖。
