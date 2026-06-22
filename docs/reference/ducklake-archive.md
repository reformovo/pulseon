# DuckLake 测试验证存档

> 存档时间：2026-06-16（初版）/ 2026-06-22（补充 PostgreSQL + 对象存储验证）
> 测试目录：`/Users/kaikai/projects/test/test-duckdb`

## 测试环境

| 项目 | 值 |
|------|-----|
| DuckDB 版本 | v1.5.3 (Variegata) `14eca11bd9` |
| DuckLake 扩展 | core 通道，catalog 版本 `1.0` |
| 平台 | darwin (macOS) |
| DuckLake 元数据 | `version=1.0`, `created_by=DuckDB 14eca11bd9`, `encrypted=false` |
| PostgreSQL（用于 catalog 验证） | 16.8 on aarch64-unknown-linux-musl (Alpine 3.20) |
| MinIO（用于对象存储验证） | `RELEASE.2025-09-07T16-13-09Z`（linux/arm64） |
| 容器运行时 | Apple 原生 `container`（Container Runtime for Linux） |

## 测试产物清单

| 产物 | 类型 | 大小 | 说明 |
|------|------|------|------|
| `metadata.ducklake` | DuckDB catalog | 9.0M | 以 DuckDB 作为 catalog 数据库，完整演练 |
| `metadata.sqlite` | SQLite catalog | 144K | 以 SQLite 作为 catalog 数据库，初始化验证 |
| `metadata.ducklake.files/` | 数据文件目录 | 6.3M | DuckLake 的 parquet 数据文件存储（本地） |
| `metadata.ducklake.files/main/test_table/` | parquet | 3.4M | `test_table` 表数据（含内联 flush） |
| `metadata.ducklake.files/main/scalar/` | parquet | 2.9M | `scalar` 表数据（含分区目录结构） |
| `swanlab-postgres` 容器内 `ducklake_catalog` 库 | PostgreSQL catalog | 8020 kB | 以 PostgreSQL 作为 catalog 数据库，初始化验证 |
| `objstore-test/objstore.ducklake` | DuckDB catalog | 5.3M | 对象存储验证：catalog 存本地，数据存 MinIO |
| `swanlab-minio` 容器内 `ducklake-test` bucket | MinIO 对象存储 | 1.0 MiB | 对象存储验证：5 个 parquet 文件（含分区目录） |

---

## 验证问题 1：DuckLake 可选用不同的数据库管理 catalog

### 结论：已验证 ✅

DuckLake 的 catalog 元数据与底层存储解耦，可插拔选择不同的数据库管理 catalog。本次实测验证了 **DuckDB**、**SQLite**、**PostgreSQL** 三种 catalog 数据库，覆盖官方推荐的全部三种场景。

### 选型指引（官方建议）

| 场景 | 推荐 catalog 数据库 | 本次验证状态 |
|------|---------------------|--------------|
| 单客户端本地数据仓库 | **DuckDB** | ✅ 完整演练 |
| 多个本地客户端的本地数据仓库 | **SQLite** | ✅ 初始化验证 |
| 多用户、可能含远程客户端的 lakehouse | **PostgreSQL** | ✅ 初始化验证（容器化） |

### 验证证据

**1. DuckDB catalog（`metadata.ducklake`）**
- 文件类型：DuckDB 数据库文件
- 完整演练：创建 2 张表、15 个快照、11 个数据文件
- 元数据表：28 张 `ducklake_*` 系统表（`ducklake_table`、`ducklake_snapshot`、`ducklake_data_file`、`ducklake_partition_column` 等）
- `ducklake_metadata` 配置：
  - `data_path = /Users/kaikai/projects/test/test-duckdb/metadata.ducklake.files/`（绝对路径）

**2. SQLite catalog（`metadata.sqlite`）**
- 文件类型：SQLite 3.x 数据库（SQLite 3038001 写入）
- 初始化验证：仅 snapshot 0（`created_schema:"main"`），未创建业务表
- 元数据表：同样包含 28 张 `ducklake_*` 系统表，schema 与 DuckDB catalog 完全一致
- `ducklake_metadata` 配置：
  - `version = 1.0`
  - `created_by = DuckDB 14eca11bd9`
  - `data_path = data_files/`（相对路径）
  - `encrypted = false`

**3. PostgreSQL catalog（容器 `swanlab-postgres` 内 `ducklake_catalog` 库）**
- 部署方式：Apple 原生 `container` 运行时（Container Runtime for Linux），非 Docker Desktop
- 容器镜像：`docker.io/library/postgres:16.8-alpine3.20`（linux/arm64）
- 容器 ID：`swanlab-postgres`，状态 `running`，地址 `192.168.64.2`，主机端口 `5432`
- PostgreSQL 版本：`16.8 on aarch64-unknown-linux-musl`（Alpine 13.2.1 gcc 编译）
- 数据卷：virtiofs 挂载 `/Users/kaikai/projects/data/swanlab/pgdata/` → `/var/lib/postgresql`
- catalog 数据库：`ducklake_catalog`（owner=swanlab，UTF8，en_US.utf8），大小 8020 kB
- 初始化验证：仅 snapshot 0（`created_schema:"main"`），未创建业务表（与 SQLite 验证同阶段）
- 元数据表：同样包含 28 张 `ducklake_*` 系统表，schema 与 DuckDB / SQLite catalog 完全一致
- `ducklake_metadata` 配置：
  - `version = 1.0`
  - `created_by = DuckDB 14eca11bd9`
  - `data_path = data_files/`（相对路径）
  - `encrypted = false`

容器列表取证（`container ls`）：

```
ID                  IMAGE                                                 OS     ARCH   STATE    ADDR
swanlab-postgres    docker.io/library/postgres:16.8-alpine3.20            linux  arm64  running  192.168.64.2
...（其他 swanlab-* 容器）
```

### 关键发现

- 三种 catalog 数据库使用**完全相同的元数据 schema**（28 张 `ducklake_*` 表），仅底层存储引擎不同。
- 切换 catalog 数据库只需更改 `ATTACH` 语句的连接串，DuckLake 上层 SQL 完全兼容。
- PostgreSQL catalog 验证了"多用户/远程客户端 lakehouse"场景的可行性：catalog 元数据集中存储在 PostgreSQL 服务端，多个 DuckDB 客户端可通过网络连接同一 catalog，实现共享 lakehouse。
- Apple 原生 `container` 运行时（非 Docker Desktop）可正常承载 PostgreSQL 作为 DuckLake catalog，对 macOS 用户是轻量化的验证/生产选项。

### 复现命令

```sql
-- 使用 DuckDB 作为 catalog
INSTALL ducklake; LOAD ducklake;
ATTACH 'metadata.ducklake' AS dl (TYPE ducklake);

-- 使用 SQLite 作为 catalog
ATTACH 'metadata.sqlite' AS dl (TYPE ducklake, CATALOG 'sqlite');

-- 使用 PostgreSQL 作为 catalog（需先启动 PostgreSQL 服务）
ATTACH 'dbname=ducklake_catalog user=swanlab password=swanlab-postgres host=127.0.0.1 port=5432' AS dl (TYPE ducklake, CATALOG 'postgres');
```

PostgreSQL 容器启动（Apple 原生 `container`）：

```bash
# 容器已在运行，可通过 container ls 查看
container ls
# 进入容器验证 catalog
container exec swanlab-postgres psql -U swanlab -d ducklake_catalog -c "\dt"
container exec swanlab-postgres psql -U swanlab -d ducklake_catalog -c "SELECT key, value FROM ducklake_metadata;"
```

---

## 验证问题 2：DuckLake 自动开启内联数据，flush 为 parquet 文件时可指定存储的目录结构

### 结论：已验证 ✅

- DuckLake 对小批量插入**自动启用内联数据（inlined data）**，无需手动配置。
- 大批量插入自动 flush 为 parquet 文件；通过**分区列（partition columns）**可指定 parquet 文件的 hive 风格目录结构。

### 验证证据

#### 2.1 内联数据自动开启

测试表 `test_table`（table_id=1，列：`id int32`, `val varchar`）与 `scalar`（table_id=2）的快照历史显示，小批量插入自动进入内联数据表：

| snapshot_id | 变更 | 说明 |
|-------------|------|------|
| 2,3,4 | `inlined_insert:1` | test_table 小批量插入 → 内联数据 |
| 8,9 | `inlined_insert:2` | scalar 小批量插入 → 内联数据 |

内联数据表（存储在 catalog 内）：

| 内联表 | table_id | schema_version | 行数 | 内容 |
|--------|----------|----------------|------|------|
| `ducklake_inlined_data_1_1` | 1 (test_table) | 1 | 6 | id 1-6，val 交替 Hello/World |
| `ducklake_inlined_data_2_2` | 2 (scalar) | 2 | 6 | scalar 初始小批量 |
| `ducklake_inlined_data_2_3` | 2 (scalar) | 3 | 0 | schema 变更后的新版本（已 flush） |

#### 2.2 大批量插入自动 flush 为 parquet

| snapshot_id | 变更 | 说明 |
|-------------|------|------|
| 5,6 | `inserted_into_table:1` | test_table 大批量插入 → flush parquet |
| 10 | `merge_adjacent:1` | 合并相邻数据文件 |
| 11,12,14 | `inserted_into_table:2` | scalar 大批量插入 → flush parquet |
| 13 | `altered_table:2` | scalar schema 变更（新增分区列） |

flush 出的 parquet 数据文件（`ducklake_data_file` 记录）：

| data_file_id | table | path | 行数 | 分区 |
|--------------|-------|------|------|------|
| 7 | test_table | `ducklake-019ecf9b-...parquet` | 200,000 | 无 |
| 8 | scalar | `ducklake-019ecf9b-d758-...parquet` | 100,000 | 无 |
| 9 | scalar | `ducklake-019ecf9c-074e-...parquet` | 100,000 | 无 |
| 10-17 | scalar | `projectId=.../experimentId=.../key=.../ducklake-*.parquet` | 500-1000 | 分区 |

#### 2.3 flush 时指定存储目录结构（分区）

`scalar` 表配置了 3 个 identity 分区列：

| partition_id | table_id | partition_key_index | column_id | 列名 | transform |
|--------------|----------|---------------------|-----------|------|-----------|
| 3 | 2 (scalar) | 0 | 3 | projectId | identity |
| 3 | 2 (scalar) | 1 | 4 | experimentId | identity |
| 3 | 2 (scalar) | 2 | 7 | key | identity |

flush 后的 parquet 文件按 **hive 风格分区目录**存储：

```
metadata.ducklake.files/main/scalar/
├── ducklake-019ecf9b-d758-75c6-9fd6-fcdac0c9f68b.parquet   # 非分区文件
├── ducklake-019ecf9c-074e-72dc-ba69-17fc17f4fd38.parquet   # 非分区文件
└── projectId=project-x/
    ├── experimentId=exp-a/
    │   ├── key=accuracy/ducklake-019ecf9e-cbf6-73e3-...parquet
    │   └── key=loss/ducklake-019ecf9e-cbf5-76b7-...parquet
    └── experimentId=exp-b/
        ├── key=accuracy/ducklake-019ecf9e-cbf6-7ca2-...parquet
        └── key=loss/ducklake-019ecf9e-cbf6-7d9a-...parquet
projectId=project-y/
    ├── experimentId=exp-a/
    │   ├── key=accuracy/ducklake-019ecf9e-cbf7-7553-...parquet
    │   └── key=loss/ducklake-019ecf9e-cbf7-7949-...parquet
    └── experimentId=exp-b/
        ├── key=accuracy/ducklake-019ecf9e-cbf7-74e4-...parquet
        └── key=loss/ducklake-019ecf9e-cbf7-7782-...parquet
```

#### 2.4 数据完整性验证

通过 DuckLake 读取验证（`ATTACH 'metadata.ducklake' AS dl (TYPE ducklake)`）：

- `test_table`：共 200,006 行 = 6 行内联 + 200,000 行 parquet；id 0-99999，val `hello-N`，内联部分含 Hello/World。
- `scalar`：共 206,006 行，跨 19 个分区组合（project-a/b/c/x/y × 多个 experiment × loss/accuracy），value 范围 0.0-0.999。

### 关键发现

- **内联数据自动启用**：小批量 INSERT 自动写入 catalog 内的 `ducklake_inlined_data_<table>_<schema_version>` 表，无需任何参数配置。
- **自动 flush 阈值**：当数据量达到阈值时，DuckLake 自动将内联数据 flush 为 parquet 文件（snapshot 5/11 等 `inserted_into_table` 事件）。
- **schema 变更触发新内联表**：`altered_table` 后会创建新的 `ducklake_inlined_data_<table>_<新schema_version>` 表（如 `_2_3`）。
- **分区目录结构**：通过 `PARTITION BY` 指定分区列后，flush 的 parquet 文件按 `列名=值/` 的 hive 目录结构存储，支持多级嵌套（projectId/experimentId/key 三级）。
- **非分区数据仍平铺**：未配置分区的表/数据文件直接存储在表目录下，文件名使用 `ducklake-<uuid>.parquet`。

### 复现命令

```sql
INSTALL ducklake; LOAD ducklake;
ATTACH 'metadata.ducklake' AS dl (TYPE ducklake);

-- 小批量插入（自动内联）
CREATE TABLE dl.test_table (id INT, val VARCHAR);
INSERT INTO dl.test_table VALUES (1, 'Hello'), (2, 'World'), ...;

-- 大批量插入（自动 flush parquet）
INSERT INTO dl.test_table SELECT range, 'hello-'||range FROM range(100000);

-- 分区表 + 指定目录结构
CREATE TABLE dl.scalar (
  id BIGINT, uid INT, projectId VARCHAR, experimentId VARCHAR,
  epoch INT, step INT, key VARCHAR, value DOUBLE,
  timestamp TIMESTAMP, createdAt TIMESTAMP
) PARTITION BY (projectId, experimentId, key);

INSERT INTO dl.scalar SELECT ...;  -- flush 时按 projectId/experimentId/key 分目录
```

---

## 验证问题 3：使用对象存储服务（MinIO / S3）存储 parquet 文件

### 结论：已验证 ✅

DuckLake 的 `data_path` 可指向 S3 兼容的对象存储服务，parquet 数据文件直接读写于对象存储上，catalog 元数据仍存储在本地数据库（DuckDB）。本次使用 Apple 原生 `container` 运行的 MinIO 完成完整验证。

### 测试环境

| 项目 | 值 |
|------|-----|
| 对象存储 | MinIO `RELEASE.2025-09-07T16-13-09Z`（linux/arm64, go1.24.6） |
| 容器 | `swanlab-minio`（Apple 原生 `container`，非 Docker Desktop） |
| 镜像 | `quay.io/minio/minio:latest` |
| 容器地址 | `192.168.64.8:9000`（S3 API），`:9001`（Console） |
| 凭据 | `MINIO_ROOT_USER=swanlab`, `MINIO_ROOT_PASSWORD=swanlab-minio` |
| 数据卷 | virtiofs 挂载 `/Users/kaikai/projects/data/swanlab/minio/data/` → `/data` |
| 测试 bucket | `ducklake-test`（本次新建） |
| DuckDB 扩展 | `ducklake` + `httpfs`（S3 协议支持） |
| catalog 数据库 | 本地 DuckDB 文件 `objstore-test/objstore.ducklake`（5.3M） |
| data_path | `s3://ducklake-test/lake/` |

### 验证证据

#### 3.1 DuckLake 创建于 S3 之上

通过 `ATTACH ... (TYPE ducklake, DATA_PATH 's3://...')` 创建 DuckLake，catalog 元数据存本地 DuckDB 文件，数据文件存 MinIO：

```sql
LOAD ducklake;
LOAD httpfs;
SET s3_endpoint='192.168.64.8:9000';
SET s3_access_key_id='swanlab';
SET s3_secret_access_key='swanlab-minio';
SET s3_url_style='path';
SET s3_use_ssl=false;

ATTACH 'objstore-test/objstore.ducklake' AS ol (TYPE ducklake, DATA_PATH 's3://ducklake-test/lake/');
```

`ducklake_metadata` 确认 `data_path = s3://ducklake-test/lake/`。

#### 3.2 内联数据 + flush parquet 到 MinIO（与本地存储行为一致）

| snapshot_id | 变更 | 说明 |
|-------------|------|------|
| 0 | `created_schema:"main"` | 初始化 |
| 1 | `created_table:"main"."test_table"` | 创建 test_table |
| 2 | `inlined_insert:1` | 小批量插入 → 内联数据（存 catalog 内） |
| 3 | `inserted_into_table:1` | 大批量插入 → flush parquet 到 MinIO |
| 4 | `created_table:"main"."scalar"` | 创建 scalar 表 |
| 5 | `altered_table:2` | `ALTER TABLE scalar SET PARTITIONED BY (...)` 设置分区 |
| 6 | `inlined_insert:2` | scalar 小批量 → 内联数据 |
| 7 | `inserted_into_table:2` | scalar 大批量 → flush parquet 到 MinIO 分区目录 |

内联数据表（存 catalog 内，与本地存储一致）：

| 内联表 | table_id | schema_version |
|--------|----------|----------------|
| `ducklake_inlined_data_1_1` | 1 (test_table) | 1 |
| `ducklake_inlined_data_2_2` | 2 (scalar) | 2 |
| `ducklake_inlined_data_2_3` | 2 (scalar) | 3（分区变更后） |

#### 3.3 MinIO 上的 parquet 文件（mc ls 取证）

flush 出的 5 个 parquet 文件实际写入 MinIO，总 1.0 MiB：

```
swanlab/ducklake-test/lake
└─ main
   ├─ scalar                                          # 分区表，hive 目录结构
   │  ├─ projectId=project-x
   │  │  ├─ experimentId=exp-a/key=loss/ducklake-*.parquet       (28 KiB, 1667 行)
   │  │  └─ experimentId=exp-b/key=loss/ducklake-*.parquet       (55 KiB, 3333 行)
   │  └─ projectId=project-y
   │     ├─ experimentId=exp-a/key=accuracy/ducklake-*.parquet   (29 KiB, 1667 行)
   │     └─ experimentId=exp-b/key=accuracy/ducklake-*.parquet   (55 KiB, 3333 行)
   └─ test_table                                      # 非分区表，平铺
      └─ ducklake-019eee8b-4d64-780e-...parquet                  (872 KiB, 100000 行)
```

`ducklake_data_file` 记录的 path 为相对路径（`path_is_relative=true`），拼接 `data_path` 后即 MinIO 上的完整 S3 路径。

#### 3.4 分区目录结构（hive 风格，与本地存储一致）

`scalar` 表通过 `ALTER TABLE ... SET PARTITIONED BY` 设置 3 级 identity 分区：

| partition_id | table_id | partition_key_index | column_id | 列名 | transform |
|--------------|----------|---------------------|-----------|------|-----------|
| 3 | 2 (scalar) | 0 | 3 | projectId | identity |
| 3 | 2 (scalar) | 1 | 4 | experimentId | identity |
| 3 | 2 (scalar) | 2 | 7 | key | identity |

flush 后 parquet 按 `projectId=.../experimentId=.../key=.../` hive 目录结构存储于 MinIO，与本地文件系统行为完全一致。

#### 3.5 数据完整性验证

- `test_table`：100,006 行 = 6 行内联 + 100,000 行 parquet（MinIO）
- `scalar`：10,002 行 = 2 行内联 + 10,000 行 parquet（MinIO），跨 5 个分区组合
- 直接从 MinIO 读 parquet 验证：`read_parquet('s3://ducklake-test/lake/main/test_table/*.parquet')` 返回 100,000 行，id 0-99999 ✅

### 关键发现

- **S3 兼容存储可用**：DuckLake 的 `data_path` 支持 `s3://` 协议，parquet 文件直接读写于 MinIO，无需额外配置。
- **需 httpfs 扩展**：访问 S3 需 `INSTALL httpfs; LOAD httpfs;` 并配置 `s3_endpoint`/`s3_access_key_id`/`s3_secret_access_key`/`s3_url_style`/`s3_use_ssl`。
- **`s3_url_style='path'`**：MinIO 需使用 path-style URL（非 virtual-hosted-style），这是与 AWS S3 的关键差异。
- **行为与本地存储完全一致**：内联数据自动开启、自动 flush parquet、分区 hive 目录结构、schema 变更等行为，在对象存储上与本地文件系统表现一致。
- **catalog 与存储解耦**：catalog 元数据存本地 DuckDB（可换 SQLite/PostgreSQL），数据文件存 MinIO，两者独立扩展。这验证了"元数据集中管理 + 数据分布式对象存储"的 lakehouse 架构。
- **分区语法**：DuckLake 不使用 `CREATE TABLE ... PARTITION BY`，而是创建表后用 `ALTER TABLE ... SET PARTITIONED BY (col1, col2, ...)` 设置分区；支持 identity/bucket/year/month/day/hour transform。
- **元数据查询**：DuckLake 的元数据表位于独立数据库 `__ducklake_metadata_<alias>`（非 schema），查询时不加 alias 前缀：`SELECT * FROM __ducklake_metadata_ol.ducklake_metadata;`；也可在 ATTACH 时用 `METADATA_CATALOG 'name'` 指定友好名称。

### 复现命令

```sql
-- 1. 配置 S3 连接
INSTALL httpfs; INSTALL ducklake;
LOAD httpfs; LOAD ducklake;
SET s3_endpoint='192.168.64.8:9000';
SET s3_access_key_id='swanlab';
SET s3_secret_access_key='swanlab-minio';
SET s3_url_style='path';
SET s3_use_ssl=false;

-- 2. 创建 DuckLake，数据存 MinIO
ATTACH 'objstore.ducklake' AS ol (TYPE ducklake, DATA_PATH 's3://ducklake-test/lake/');

-- 3. 内联 + flush
CREATE TABLE ol.test_table (id INT, val VARCHAR);
INSERT INTO ol.test_table VALUES (1,'Hello'),(2,'World');           -- 内联
INSERT INTO ol.test_table SELECT range, 'hello-'||range FROM range(100000);  -- flush 到 MinIO

-- 4. 分区表
CREATE TABLE ol.scalar (id BIGINT, projectId VARCHAR, experimentId VARCHAR, key VARCHAR, value DOUBLE);
ALTER TABLE ol.scalar SET PARTITIONED BY (projectId, experimentId, key);    -- DuckLake 分区语法
INSERT INTO ol.scalar SELECT ...;                                   -- flush 到 MinIO hive 目录

-- 5. 查询元数据（注意：__ducklake_metadata_ol 是数据库，不加 ol. 前缀）
SELECT * FROM __ducklake_metadata_ol.ducklake_data_file;
SELECT * FROM __ducklake_metadata_ol.ducklake_partition_column;
```

MinIO 侧验证：

```bash
mc alias set swanlab http://192.168.64.8:9000 swanlab swanlab-minio
mc ls --recursive swanlab/ducklake-test/lake    # 列出 parquet 文件
mc tree swanlab/ducklake-test/lake              # 查看分区目录结构
```

### 补充验证：主动 flush 内联数据到对象存储

#### 背景

小批量 INSERT 自动进入内联数据（存 catalog 内），不会立即写入对象存储。需主动调用 flush 函数将内联数据落盘为 parquet。

#### flush 函数：`ducklake_flush_inlined_data`

```sql
-- flush 所有内联数据
CALL ducklake_flush_inlined_data('ol');

-- flush 指定 schema
CALL ducklake_flush_inlined_data('ol', schema_name => 'main');

-- flush 指定表
CALL ducklake_flush_inlined_data('ol', table_name => 'test_table');

-- flush 指定 schema 的指定表
CALL ducklake_flush_inlined_data('ol', schema_name => 'main', table_name => 'scalar');
```

返回值（每行一个被 flush 的表，无内联数据的表不返回）：

| 列 | 类型 | 说明 |
|----|------|------|
| `schema_name` | VARCHAR | schema 名 |
| `table_name` | VARCHAR | 表名 |
| `rows_flushed` | BIGINT | 从内联转为 parquet 的行数 |

> `CHECKPOINT ol;` 也会内部调用 `ducklake_flush_inlined_data`，效果相同。

#### 实测验证（MinIO）

**flush 前**：
- 内联数据：test_table 6 行、scalar 2 行（存 catalog 内）
- MinIO 上 parquet 文件：5 个

**执行 flush**：
```sql
CALL ducklake_flush_inlined_data('ol');
-- 返回：
--   main | scalar     | 2
--   main | test_table | 6
```

**flush 后**：
- 快照新增 `snapshot 8: inline_flush:1,inline_flush:2`（两张表的内联数据被 flush）
- MinIO 上 parquet 文件：5 → **8 个**（新增 3 个小文件）
  - `test_table/ducklake-019eee96-...parquet`（795 B，6 行）
  - `scalar/projectId=project-x/experimentId=exp-a/key=loss/ducklake-019eee96-...parquet`（1.6 KiB，1 行）
  - `scalar/projectId=project-x/experimentId=exp-a/key=accuracy/ducklake-019eee96-...parquet`（1.7 KiB，1 行）
- 分区表的 flush 同样遵循 hive 目录结构，内联的 2 行 scalar 按 `projectId/experimentId/key` 分到对应分区目录
- 数据完整性不变：test_table 100,006 行、scalar 10,002 行 ✅

#### 内联数据阈值配置

内联数据的自动 flush 阈值由 `DATA_INLINING_ROW_LIMIT` 控制（默认 10 行）：插入行数 < 阈值则内联，≥ 阈值则直接写 parquet。

```sql
-- 全局默认（0 = 禁用内联，所有插入直接写 parquet）
SET ducklake_default_data_inlining_row_limit = 50;

-- ATTACH 时指定
ATTACH 'objstore.ducklake' AS ol (
    TYPE ducklake,
    DATA_PATH 's3://ducklake-test/lake/',
    DATA_INLINING_ROW_LIMIT 10
);

-- 持久化到 DuckLake metadata（按表）
CALL ol.set_option('data_inlining_row_limit', 10, table_name => 'test_table');
```

#### 关键发现

- **主动 flush**：`CALL ducklake_flush_inlined_data('alias')` 将内联数据写入 parquet（对象存储/本地），返回每表 flush 行数。
- **分区感知**：flush 分区表的内联数据时，自动按 hive 目录结构写入对应分区。
- **快照记录**：flush 产生 `inline_flush:<table_id>` 快照变更记录。
- **CHECKPOINT 等价**：`CHECKPOINT alias` 内部调用 flush，效果相同。
- **阈值可调**：`DATA_INLINING_ROW_LIMIT` 控制内联 vs 直接写 parquet 的界限，设为 0 可禁用内联。
- **小文件问题**：频繁小批量 + flush 会产生大量小 parquet 文件（如本次 6 行仅 795 B），生产环境建议配合 `auto_compact` 或定期 compact 合并。

#### flush 粒度：不支持分区级

`ducklake_flush_inlined_data` 仅支持 `schema_name` 和 `table_name` 两个参数（源码确认），**不支持按分区值过滤**。内联数据表 `ducklake_inlined_data_<table>_<version>` 将所有分区的行存在同一张表里，flush 时读取全部行写入 parquet，无法只 flush 某个实验的数据。

考虑过的替代方案及实测评估：

| 方案 | 做法 | 评估 |
|------|------|------|
| 禁用内联（`data_inlining_row_limit=0`） | 所有 INSERT 直接写 parquet | ❌ 逐条插入产生大量小文件（见下方实测） |
| DELETE + flush + 重新导入 | 备份目标分区 → 删除 → flush → 重新导入 | ⚠️ 丢失快照历史，步骤复杂 |
| **保持内联 + 实验结束时全表 flush** | 小批量走内联，实验结束调用 flush | ✅ **推荐** |

#### 禁用内联的小文件实测（`data_inlining_row_limit = 0`）

对 `scalar` 表禁用内联后测试三种插入模式：

| 测试 | 操作 | 产生文件数 | 文件大小 |
|------|------|-----------|----------|
| 3 次单行 INSERT | `INSERT ... VALUES (1行)` × 3 | **3 个**（各 1 行） | 各 1.3 KiB |
| 1 次多行 INSERT（同分区） | `INSERT ... VALUES (3行)` × 1 | **1 个**（3 行） | 1.4 KiB |
| 1 次多行 INSERT（跨 3 分区） | `INSERT ... VALUES (3行, 3个分区)` × 1 | **3 个**（各 1 行） | 各 1.3 KiB |

**文件数公式**：`INSERT 语句数 × 每语句涉及的分区数`

> 逐条插入场景（如训练过程每个 step 发一次 INSERT）：10000 个 step = 10000 个 1.3 KiB parquet 文件，完全丧失 parquet 列式批量读取的优势。**禁用内联不适合逐条插入场景。**

#### 推荐方案：保持内联 + 实验结束时 flush

对实验追踪场景（`scalar` 表按 `projectId, experimentId, key` 分区），推荐：

1. **保持默认内联**（`data_inlining_row_limit` 默认值 10）：训练过程中的小批量指标记录走内联，存 catalog 内，不产生小文件。
2. **实验结束时执行 flush**：将整个实验期间累积的内联数据一次性落盘为 parquet，文件按 hive 分区目录组织。
3. **定期 compact 合并**（可选）：flush 后若小文件较多，合并同分区文件。

```sql
-- 实验结束时执行（全表 flush，落盘的文件天然按分区目录隔离）
CALL ducklake_flush_inlined_data('ol', table_name => 'scalar');

-- 可选：合并小文件
CALL ducklake_merge_adjacent_files('ol', 'scalar', max_compacted_files => 10);
```

**优势**：
- 训练过程中无小文件问题（数据在内联表中）
- 实验结束时一次性 flush，每个分区的数据合并为一个 parquet 文件
- flush 是全表的，但落盘文件按 `projectId/experimentId/key` 分区目录隔离，查询时分区裁剪正常
- 实现简单，无需应用层改动

---

## 附：DuckLake catalog 元数据 schema

三种 catalog 数据库（DuckDB / SQLite / PostgreSQL）均包含以下 28 张系统表，schema 完全一致：

```
ducklake_column                  ducklake_file_partition_value
ducklake_column_mapping          ducklake_file_variant_stats
ducklake_column_tag              ducklake_files_scheduled_for_deletion
ducklake_data_file               ducklake_inlined_data_tables
ducklake_delete_file             ducklake_macro
ducklake_file_column_stats       ducklake_macro_impl
ducklake_macro_parameters        ducklake_snapshot_changes
ducklake_metadata                ducklake_sort_expression
ducklake_name_mapping            ducklake_sort_info
ducklake_partition_column        ducklake_table
ducklake_partition_info          ducklake_table_column_stats
ducklake_schema                  ducklake_table_stats
ducklake_schema_versions         ducklake_tag
ducklake_snapshot                ducklake_view
```

业务表（本次测试）：

| table_id | 表名 | 路径 | 列 |
|----------|------|------|----|
| 1 | test_table | test_table/ | id(int32), val(varchar) |
| 2 | scalar | scalar/ | id(int64), uid(int32), projectId(varchar), experimentId(varchar), epoch(int32), step(int32), key(varchar), value(float64), timestamp(timestamp), createdAt(timestamp) |
