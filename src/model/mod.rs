// model/mod.rs — 逻辑数据模型（纯数据类型，无 I/O 依赖）
// Architecture ref: docs/native-architecture.md §3, §6.1

pub mod types;
pub mod run;
pub mod metric;
pub mod event;
pub mod summary;
pub mod config;
pub mod tag;
pub mod artifact;
