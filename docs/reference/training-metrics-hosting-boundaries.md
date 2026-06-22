
> MotherDuck 这张 `catalog / storage / compute` 分层表，我越看越觉得它不是在卖套餐，而是在替 lakehouse 产品划边界。它真正告诉我的不是“怎么托管 DuckLake”，而是“如果你要把一套数据协议产品化，哪些层必须允许独立托管、独立替换、独立收费”。这对训练指标产品尤其重要，因为我们天然就要面对本地、云端、私有化三种落点。

## 先承认：产品卖的不是数据库，而是托管边界

MotherDuck 的三档设计看起来很像 SKU，实际上是在定义控制权：

- `Fully Managed`：平台托管 `catalog`、`storage`、`compute`
- `Bring-Your-Own-Bucket`：平台托管 `catalog` 和 `compute`，用户自带存储
- `Bring-Your-Own-Compute`：平台只保留 `catalog`，其余都可替换

这套分层的价值，在于它把“我到底拥有哪一层”说得非常清楚。以前很多 lakehouse 讨论只停留在格式层，最后往往会在部署时暴露出真正的问题：数据文件谁管，执行引擎谁管，权限和事务谁管，出问题时谁来兜底。

对我来说，这里最重要的启发是：**托管形态不是产品的附属包装，而是架构本身的一部分。**

## 对训练指标产品的映射

我们这套训练指标架构，恰好也能自然切成三层：

- `Catalog`：`run`、`metric`、`file`、`snapshot`、权限、artifact 索引
- `Storage`：`Parquet + zstd` 文件、对象存储或本地磁盘
- `Compute`：`DuckDB` 本地分析、`ClickHouse` 在线 serving、未来可替换的其他引擎

这意味着，我们不该把“本地版 / 云端版 / 私有化版”理解成三套独立系统，而该理解成三种托管边界的组合。

## 我会怎么划三档

### 1. 全托管

适合想“开箱即用”的团队。

- `Catalog`：平台托管
- `Storage`：平台托管
- `Compute`：平台托管

这时用户只关心写入和看板，不关心文件位置和查询引擎。产品价值是最低心智负担。

### 2. 自带 Bucket

适合重视数据主权、合规和迁移自由的团队。

- `Catalog`：平台托管
- `Storage`：用户自带
- `Compute`：平台托管

这是我认为最值得优先支持的中间态。它保留了平台的 query 和 catalog 能力，同时把事实数据放回用户控制的对象存储里。对训练指标来说，这几乎就是“数据可带走、体验不打折”的最佳平衡。

### 3. 自带 Compute

适合已经有自己分析栈、或者明确希望多引擎复用同一数据协议的团队。

- `Catalog`：平台托管或用户自管
- `Storage`：用户自带
- `Compute`：用户自带

这一档的本质，不是平台放弃价值，而是平台把价值重心收缩到协议和元数据管理上。对我们来说，这正好对应“同一份 `Parquet` 既能给 DuckDB 读，也能给 ClickHouse 导入”的目标。

## 对我们架构的直接启发

### 1. `catalog` 必须是一等公民

MotherDuck 的表述让我更确定：真正难迁移的不是文件，而是控制面。

所以训练指标产品里，最该精细打磨的是：

- `files` 怎么注册
- `snapshots` 怎么提交
- `run` 怎么查
- `summary` 怎么更新
- 权限和可见性怎么跟着 catalog 走

如果这层不稳，`Parquet` 再开放也只是把问题往后推。

### 2. `storage` 应该天然可替换

我们已经把事实数据定成 `Parquet + zstd`，这就意味着：

- 本地可以是磁盘目录
- 云端可以是对象存储
- 私有化可以是用户自己的 bucket

这不是实现细节，而是产品契约的一部分。只要格式开放，用户就能在不同托管边界之间迁移。

### 3. `compute` 不该绑死一个引擎

MotherDuck 明确把 compute 当成可替换层，这给了我们一个很强的方向感：

- 本地优先 DuckDB
- 在线优先 ClickHouse
- 如果未来有别的引擎，只要吃同一协议也可以接进来

这也是为什么我不想把系统定义成“一个数据库产品”，而想定义成“一个训练指标协议 + 多种执行方式”。

## 一个更实际的结论

如果把 MotherDuck 的思路压缩成一句对我们有用的话，那就是：**先把控制权切成层，再决定每一层由谁托管。**

对训练指标产品来说，这会直接导向一个非常清晰的产品策略：

- 先做统一协议和统一 catalog
- 再提供全托管 / 自带存储 / 自带计算三种交付方式
- 最后用同一套前端和查询接口把这些模式连起来

这比“先做一个最强数据库，再想怎么包装成产品”更符合我们要做的东西。

来源：[[sources/announcing-ducklake-1-0-on-motherduck]] · [[sources/ducklake-v1-0-announcement]] · [[sources/ducklake-manifesto]]

相关页面：[[topics/training-metrics-storage-architecture]] · [[topics/ducklake]] · [[entities/motherduck]] · [[entities/duckdb]] · [[entities/ducklake]]
