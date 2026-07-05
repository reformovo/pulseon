use std::path::{Path, PathBuf};

use crate::engine::EngineError;

pub(crate) struct NativeStorageConfig {
    catalog_path: PathBuf,
    data_path: PathBuf,
}

impl NativeStorageConfig {
    pub(crate) fn duckdb(
        root_path: &Path,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        let pulseon_path = root_path.join(".pulseon");
        Self {
            catalog_path: catalog_path.unwrap_or_else(|| pulseon_path.join("catalog.ducklake")),
            data_path: data_path.unwrap_or_else(|| pulseon_path.join("data")),
        }
    }
}

#[cfg(test)]
pub(crate) fn open_native_connection(root_path: &Path) -> Result<duckdb::Connection, EngineError> {
    open_native_connection_with_config(NativeStorageConfig::duckdb(root_path, None, None))
}

pub(crate) fn open_native_connection_with_config(
    config: NativeStorageConfig,
) -> Result<duckdb::Connection, EngineError> {
    if let Some(catalog_parent) = config.catalog_path.parent() {
        std::fs::create_dir_all(catalog_parent).map_err(|source| EngineError::Storage {
            operation: "creating catalog directory",
            name: path_basename(catalog_parent),
            source,
        })?;
    }
    std::fs::create_dir_all(&config.data_path).map_err(|source| EngineError::Storage {
        operation: "creating data directory",
        name: path_basename(&config.data_path),
        source,
    })?;

    let connection = open_duckdb_connection()?;
    attach_ducklake(&connection, &config.catalog_path, &config.data_path)?;
    create_v1_tables(&connection)?;
    Ok(connection)
}

pub(crate) fn attach_ducklake(
    connection: &duckdb::Connection,
    catalog_path: &Path,
    data_path: &Path,
) -> Result<(), EngineError> {
    let catalog_path = sql_string_literal(catalog_path.to_string_lossy().as_ref());
    let data_path = sql_string_literal(data_path.to_string_lossy().as_ref());
    connection.execute_batch(&format!(
        "INSTALL ducklake;
         LOAD ducklake;
         ATTACH {catalog_path} AS dl (TYPE ducklake, DATA_PATH {data_path});"
    ))?;
    Ok(())
}

fn open_duckdb_connection() -> Result<duckdb::Connection, EngineError> {
    let mut config = duckdb::Config::default();
    if std::env::var_os("PULSEON_LTTB_EXTENSION_PATH").is_some() {
        config = config.allow_unsigned_extensions()?;
    }
    Ok(duckdb::Connection::open_in_memory_with_flags(config)?)
}

pub(crate) fn create_v1_tables(connection: &duckdb::Connection) -> Result<(), EngineError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS dl.pulseon_projects (
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dl.pulseon_runs (
             run_id VARCHAR NOT NULL,
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             status VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL,
             started_at TIMESTAMPTZ NOT NULL,
             finished_at TIMESTAMPTZ
         );
         CREATE TABLE IF NOT EXISTS dl.metric_points (
             run_id VARCHAR NOT NULL,
             metric_key VARCHAR NOT NULL,
             step BIGINT NOT NULL,
             timestamp TIMESTAMPTZ NOT NULL,
             value_f64 DOUBLE NOT NULL,
             ingested_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dl.pulseon_metric_aggregates (
             run_id VARCHAR NOT NULL,
             metric_key VARCHAR NOT NULL,
             effective_count UBIGINT NOT NULL,
             last_step BIGINT NOT NULL,
             last_value_f64 DOUBLE NOT NULL,
             min_value_f64 DOUBLE NOT NULL,
             max_value_f64 DOUBLE NOT NULL
         );",
    )?;
    Ok(())
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("storage path")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_string_literal_escapes_single_quotes() {
        assert_eq!(sql_string_literal("canary's/data"), "'canary''s/data'");
    }
}
