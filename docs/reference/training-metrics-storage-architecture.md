
> 如果把一个类似 Weights & Biases 或 SwanLab 的产品只理解成"记录 loss 曲线"，后面的架构判断几乎一定会做小。我现在更倾向把它看成一条完整的数据链路：训练进程持续写入，用户希望本地零部署可用，团队版本又希望云端托管与私有化共存，而且分析看板最好还是纯前端。真正难的地方不是选 DuckDB 还是 ClickHouse，而是定义一套能跨三种部署形态复用的物理模型。

## 要实现的需求，不是一套数据库，而是一套可迁移的数据协议

这类产品至少要同时满足三种使用方式：

1. 完全本地化，不额外部署本地服务。
2. 上报到云端服务，由平台托管存储和查询。
3. 私有化部署，在用户自己的基础设施里运行。

我会把这三个模式看成同一产品的三个落点，而不是三套独立实现。原因很现实：如果本地版写一种格式，云端版写另一种格式，私有化版再维护第三种格式，前端、导入导出、迁移和长期兼容成本都会迅速失控。

因此，这份架构的真正目标不是"找一个最快的数据库"，而是定义四件事：

- 什么是统一的逻辑数据模型。
- 什么是统一的交换物理格式。
- 元信息和时序指标分别放在哪里。
- DuckDB、ClickHouse、纯前端看板分别在系统里扮演什么角色。

## 必须先拆开的四个问题

我会先把问题拆成四层，否则讨论很容易混成一句空话，比如"直接用 ClickHouse"或者"本地就用 DuckDB"。

### 1. 写入路径和查询路径不是同一个问题

训练指标写入天然是 append-only、小批量、高频；查询路径则是折线图、run 对比、summary 聚合、事件回看。这两个负载的优化目标不一样。写入路径最怕小文件和频繁 schema 演化，查询路径最怕扫太多无关数据。

### 2. 元信息和事实数据的物理特征完全不同

`workspace`、`project`、`run`、`config`、`tag`、权限和 artifact 索引这类元信息，数据量不大，但更新频繁、需要事务一致性。指标点和事件流则正相反：数据量大，主要追加，适合列式扫描。如果把两者强塞进同一种存储介质里，往往两边都做不好。

### 3. 本地零部署与云端在线服务的约束相反

本地模式最重要的是"不需要守护进程"，因此我不希望它依赖额外的本地数据库服务。云端模式则相反，它要的是多租户、并发写入、稳定的在线查询服务，这正是长驻分析数据库擅长的事情。

### 4. 纯前端看板不能绑定某个底层数据库

如果分析看板未来想做成纯前端，它就不应该直接耦合某个特定存储引擎，而应该绑定统一的查询接口或数据适配层。否则本地版和云端版会在 UI 层分叉。

## 核心决策：统一的是数据协议，不是执行引擎

这是整份设计里最重要的判断。

我不会追求"所有场景都直接查同一份物理副本"。更稳的目标是统一三层：

- 统一逻辑模型：run、metric、event、summary、artifact 这些对象的含义一致。
- 统一交换格式：跨场景可落地、可迁移、可导入导出的事实数据格式一致。
- 统一 catalog 模型：文件、schema、快照和元信息的管理方式一致。

执行引擎则允许按场景变化：

- 本地分析可以直接用 DuckDB。
- 云端在线分析可以把同一批事实数据导入 ClickHouse serving 表。
- 前端只认统一查询接口，不认底层数据库名字。

这个边界很关键。它避免了"为了兼容所有场景而把每个场景都做差"。

## 决策一：统一逻辑模型采用 run-centric 设计

我会把系统中的核心对象固定为下面几类：

- `workspace`
- `project`
- `run`
- `metric_definition`
- `metric_point`
- `run_event`
- `run_summary`
- `artifact`
- `config`
- `tag`

这里最重要的不是命名，而是约束抽象层级。无论本地、云端还是私有化，都应该围绕 `run` 组织指标和查询，而不是围绕某个底层表名或存储路径组织产品概念。

我特别希望把 `metric_definition` 和 `metric_point` 分开，因为它能把"这个指标是什么"和"这个指标每一步的值"拆开。前者属于元信息层，后者属于事实层。这样 schema 更稳，也更适合前端做指标发现和自动补全。

## 决策二：统一交换物理格式选 `Parquet + zstd`

如果只看跨场景复用，我认为这里几乎没有第二个同等级选项。

`Parquet` 的价值不在于它时髦，而在于它同时满足两端：

- 对 DuckDB 来说，它是最自然的本地 / 对象存储分析格式，列裁剪和 row group pruning 都能直接利用。
- 对 ClickHouse 来说，它非常适合作为导入导出和长期交换格式，可以通过 `file()`、`s3()`、`url()` 等路径直接读取，再写入 MergeTree 系列表。

这件事和 [[topics/clickhouse-data-export]] 里的判断是一致的：

- 面向 ClickHouse 内部回灌，`Native + zstd` 更快。
- 面向跨系统共享、用户持有和长期归档，`Parquet + zstd` 更稳。

所以我会把 `Native` 视为系统内部优化通道，而把 `Parquet` 视为产品对外承诺的标准物理格式。

## 决策三：元信息放 SQL catalog，不放进 Parquet 文件森林

这是我从 [[topics/ducklake]] 里吸收最多的一点，但我不会直接把产品绑定到 DuckLake 本身。

我的判断是：**元数据管理本质上是数据库事务问题，不是文件命名问题。**

因此，统一格式不应该只是一个目录里堆一堆 `manifest.json`、`snapshot.json` 和 Parquet 文件。我更倾向的结构是：

- 数据文件仍然是开放的 `Parquet`
- catalog 元信息由 SQL 数据库管理

这层 catalog 在不同部署形态下可以换后端：

- 本地零部署：`SQLite`
- 云端 / 私有化：`Postgres`

但逻辑表结构应该尽量一致。至少需要这些表：

- `workspaces`
- `projects`
- `runs`
- `metric_definitions`
- `files`
- `snapshots`
- `artifacts`
- `run_summaries`
- `tags`

其中最关键的是 `files` 和 `snapshots`。它们共同定义了：当前数据集由哪些文件构成、每个文件覆盖哪些时间 / step 范围、当前可读快照是哪一版。

一个足够实用的 `files` 表会包含这些字段：

```text
file_id
dataset_id
table_name
path
format
compression
row_count
min_step
max_step
min_timestamp
max_timestamp
project_id
run_id
metric_name
schema_version
content_hash
created_at
```

我之所以坚持 catalog 入库，而不是继续堆 manifest 文件，原因很简单：一旦你需要原子提交、并发控制、文件注册、schema 版本和小写入管理，你本质上已经在做数据库该做的事。那还不如直接承认这一点。

## 决策四：时序指标采用长表，而不是动态宽表

训练指标最容易让人误入的歧路，是把每个 metric 当成一列拼成宽表。这个方案在 demo 阶段看起来很直观，但一旦面对用户自定义 metric、稀疏记录、不同 run 指标集合不一致，它会迅速变得难维护。

我更倾向的主事实表是 `metric_points` 长表：

```text
workspace_id   STRING
project_id     STRING
run_id         STRING
metric_name    STRING
step           BIGINT
timestamp      TIMESTAMP
value_f64      DOUBLE
value_i64      BIGINT
value_str      STRING
value_bool     BOOLEAN
value_type     STRING
phase          STRING
rank           INTEGER
source         STRING
```

这里我故意没有把值直接塞进一个 JSON 里。理由有三条：

1. DuckDB 对列式 typed column 的利用远好于对大块 JSON 的临时解析。
2. ClickHouse 导入后也更容易映射成稳定 schema。
3. 折线图的热路径本来就只关心 `run_id + metric_name + step/timestamp + value`，不值得为了抽象上的整齐牺牲热路径性能。

并列的辅助事实表则至少还需要两张：

- `run_events`：记录 checkpoint、状态变化、告警、异常。
- `run_summary`：记录最后值、最优值、最佳 step、聚合摘要。

我会把它理解成一条主线和两条旁路：

- `metric_points` 服务于图表查询
- `run_events` 服务于时间线和诊断
- `run_summary` 服务于列表页、筛选和排序

## 决策五：物理目录按 run 和 metric 组织，而不是按时间粗暴分桶

很多时序系统天然会先想到按天或小时分区，但训练指标的主查询模式并不是"看某一天全局发生了什么"，而是"看某个 run 的一组指标如何演化"。因此，训练指标更适合先按业务裁剪维度组织，再在文件内部按 step 或 timestamp 保序。

我会建议 v1 目录结构长这样：

```text
dataset/
  catalog.sqlite
  metric_points/
    project=<project_id>/
      run=<run_id>/
        metric=<metric_name>/
          part-000001.parquet
          part-000002.parquet
  run_events/
    project=<project_id>/
      run=<run_id>/
        part-000001.parquet
  run_summary/
    project=<project_id>/
      run=<run_id>/
        part-000001.parquet
  artifacts/
    project=<project_id>/
      run=<run_id>/
        ...
```

这个结构背后的逻辑很直接：

- 本地 DuckDB 扫描目录很自然。
- ClickHouse 从对象存储导入时也容易根据 catalog 拿到明确文件列表。
- 单条图表查询通常只需要命中一个 run 下某几个 metric 文件，不会误扫全局。

我不会在 v1 里过早引入复杂的时间分区层级。训练指标不是日志平台，先按 `run` 裁剪通常比按日期裁剪更有效。

## 决策六：小写入先进入 staging，再批量 flush 成 Parquet

这是实现里最不能偷懒的部分。

如果 SDK 每上报几个点就立刻写一个 Parquet 文件，系统很快会被小文件问题拖垮。DuckLake 的 Data Inlining 给了我一个很强的工程提醒：**小写入不应该直接变成对象存储上的独立文件。**

但我这里不会直接复刻 DuckLake 的完整机制，而是做一个更专用的 staging 层：

- 本地模式：先写入 `SQLite` staging 表或本地 WAL
- 云端 / 私有化：先写入服务端接收缓冲，再异步 flush 成批量 Parquet

flush 规则应该明确，而不是靠感觉：

- 达到行数阈值再 flush
- 达到时间窗口再 flush
- run 结束时强制 flush
- 后台 compaction 把碎片文件合并到目标大小

我会把单个 Parquet 文件目标控制在 `64 MB - 256 MB`。这是一个比较折中的区间：够大，避免小文件泛滥；又不至于大到单次查询为了拿一条曲线就扫太多无关数据。

## 决策七：本地模式直接查 Parquet，云端模式把 Parquet 导入 ClickHouse serving 表

这一步是 DuckDB 和 ClickHouse 真正分工的地方。

### 本地模式

本地模式的关键不是"也做一个迷你服务端"，而是让用户在没有额外守护进程的前提下完成记录和分析。所以我更倾向：

- SDK 写入本地 catalog + Parquet
- 查询直接由 DuckDB 执行
- 如果是 Web 看板，本地查询层可以考虑 `DuckDB-WASM`

这个模式的优点是简单、便携、零部署。它天然适合个人实验和小团队离线分析。

### 云端 / 私有化模式

云端和私有化模式里，我不会让 DuckDB 直接承担长期在线多租户查询服务。这里更稳的结构是：

- `Postgres` 管应用元信息和 catalog
- `Parquet` 管共享事实数据和长期交换格式
- `ClickHouse` 管在线分析 serving 副本

写入路径可以是：

1. SDK 上报到接收层
2. 接收层先写 catalog / staging
3. 批量 flush 为 `Parquet`
4. 异步把新文件导入 ClickHouse
5. 前端查询在线走 ClickHouse，导出和离线分析仍然拿 Parquet

这样做的关键好处是：统一数据层不等于强迫每个查询都直接扫数据湖。在线产品应该有自己的 serving 副本。

## 决策八：前端看板绑定统一查询接口，不绑定底层存储

如果分析看板最终要做成纯前端，我会要求它只依赖统一的适配接口，例如：

- `listRuns(projectId)`
- `listMetrics(runId)`
- `queryMetricSeries(runIds, metricNames, range)`
- `queryRunSummary(filters, sort)`
- `listRunEvents(runId, range)`

本地版、云端版、私有化版都实现同一组能力，但后端来源不同：

- `LocalAdapter` 走本地 catalog + DuckDB
- `CloudAdapter` 走远端 API + ClickHouse
- `SelfHostedAdapter` 走私有部署 API

这个抽象看起来普通，但它实际上保护了 UI 层不被底层存储分叉带走。

## 为什么不直接依赖 DuckLake

我现在的结论是：**不直接依赖 DuckLake 作为底层协议，但明确借鉴它的设计逻辑。**

原因有三条。

第一，产品的兼容对象不只有 DuckDB，还有 ClickHouse。DuckLake 对 DuckDB 很自然，但它不会自动降低 ClickHouse 侧的导入和 serving 复杂度。

第二，这里要解决的是"训练指标专用 catalog"，不是通用 lakehouse。通用 lakehouse 要处理更复杂的 schema 演化、删除向量、多 writer 并发、跨引擎兼容。训练指标场景的约束强得多，没有必要一开始就买下整套复杂度。

第三，DuckLake 最有价值的东西其实不是它的名字，而是几条判断：

- 元数据进 SQL，而不是进 JSON 文件森林
- 小写入先内联 / staging，再 flush
- 数据文件保持开放的 Parquet
- 排序、分桶和统计信息在写入路径提前做掉

这些判断完全可以在不依赖 DuckLake 运行时的情况下自己实现，而且复杂度是可控的。

## 我会怎么定义 v1 的实现边界

如果把这个架构压成一个可落地的 v1，我会刻意收窄范围：

- 单 writer 为主，不做复杂的多 writer 并发提交协议
- 事实数据以 append-only 为主
- 不支持通用 `DELETE` / `UPDATE`
- `run_summary` 可以更新，但通过 catalog / side table 完成
- catalog 后端只支持 `SQLite` 和 `Postgres`
- 数据文件只承诺 `Parquet + zstd`
- serving 层只优先支持 `DuckDB` 与 `ClickHouse`

这样做不是保守，而是把复杂度放在真正有产品价值的地方：统一格式、查询体验、迁移路径，而不是过早重做一个通用数据湖协议。

## 被我明确拒绝的几条路径

### 1. 本地也强制部署服务端

这会直接破坏"零额外服务"这个核心约束。只要本地用户需要再起一个 daemon，整个产品体验就已经偏离目标。

### 2. 所有数据都塞进一个本地数据库文件

这条路短期最省事，但会让云端 / 私有化的迁移和共享格式变得别扭。它也不利于对象存储、跨系统分析和长周期归档。

### 3. 所有时序指标都写成 JSON blob

这看起来灵活，实际上是在把最常见的查询路径做慢。半结构化信息可以存在侧边，但热路径不能建立在大块 JSON 解析之上。

### 4. 一开始就追求通用 lakehouse 兼容

这会把问题从"实现训练指标产品"变成"重新设计一个面向任意引擎的数据协议"。对当前目标来说，这个抽象层级太高了。

## 最终架构判断

如果我把整份文档压成一句话，它会是：**用 `Parquet + SQL catalog` 作为统一数据协议，用 DuckDB 解决本地零部署分析，用 ClickHouse 解决云端在线 serving，用同一套 run-centric 逻辑模型把三种部署形态接起来。**

这套设计最让我满意的地方，不是它追求单点最优，而是它把几个经常互相冲突的目标重新对齐了：

- 本地可以零部署
- 云端可以做在线服务
- 私有化可以沿用同一协议
- 用户始终持有开放的事实数据格式
- 前端不需要知道底层到底是 DuckDB 还是 ClickHouse

对我来说，这才像一个能长期长大的产品底座。

来源：[[sources/duckdb-vs-clickhouse-posthog]] · [[sources/ducklake-manifesto]] · [[sources/ducklake-v1-0-announcement]] · [[sources/oneuptime-clickhouse-export-file-formats]]

相关页面：[[topics/duckdb-vs-clickhouse]] · [[topics/ducklake]] · [[topics/clickhouse-data-export]] · [[entities/duckdb]] · [[entities/clickhouse]] · [[entities/ducklake]] · [[entities/posthog]]
