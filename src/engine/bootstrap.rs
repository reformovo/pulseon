use std::path::{Path, PathBuf};

use crate::engine::EngineError;

const DUCKLAKE_ALIAS: &str = "dl";
const DUCKDB_CATALOG_ALIAS: &str = "pulseon_catalog";

pub(crate) struct NativeStorageConfig {
    catalog_path: PathBuf,
    data_path: PathBuf,
}

struct CatalogAdapter {
    default_catalog_filename: &'static str,
    ducklake_alias: &'static str,
    catalog_application_database: &'static str,
}

impl CatalogAdapter {
    const fn duckdb() -> Self {
        Self {
            default_catalog_filename: "catalog.ducklake",
            ducklake_alias: DUCKLAKE_ALIAS,
            catalog_application_database: DUCKDB_CATALOG_ALIAS,
        }
    }

    fn attach_ducklake_statement(&self, catalog_path: &Path, data_path: &Path) -> String {
        let catalog_path = sql_string_literal(catalog_path.to_string_lossy().as_ref());
        let data_path = sql_string_literal(data_path.to_string_lossy().as_ref());
        format!(
            "ATTACH {catalog_path} AS {} (
                 TYPE ducklake,
                 DATA_PATH {data_path},
                 METADATA_CATALOG '{}'
             );",
            self.ducklake_alias, self.catalog_application_database
        )
    }

    fn setup_catalog_application_tables(
        &self,
        connection: &duckdb::Connection,
        _catalog_path: &Path,
    ) -> Result<(), EngineError> {
        connection
            .execute_batch(&format!("USE {};", self.catalog_application_database))
            .map_err(|source| EngineError::StorageDuckDb {
                operation: "selecting PulseOn catalog tables",
                name: self.catalog_application_database.to_owned(),
                source,
            })?;
        Ok(())
    }
}

impl NativeStorageConfig {
    pub(crate) fn duckdb(
        root_path: &Path,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        let pulseon_path = root_path.join(".pulseon");
        let adapter = CatalogAdapter::duckdb();
        Self {
            catalog_path: catalog_path
                .unwrap_or_else(|| pulseon_path.join(adapter.default_catalog_filename)),
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
    setup_duckdb_catalog_adapter(&connection, &config.catalog_path)?;
    create_v1_tables(&connection)?;
    Ok(connection)
}

pub(crate) fn attach_ducklake(
    connection: &duckdb::Connection,
    catalog_path: &Path,
    data_path: &Path,
) -> Result<(), EngineError> {
    let storage_name = format!(
        "{}, {}",
        path_basename(catalog_path),
        path_basename(data_path)
    );
    let adapter = CatalogAdapter::duckdb();
    connection
        .execute_batch(&format!(
            "INSTALL ducklake;
         LOAD ducklake;
         {}",
            adapter.attach_ducklake_statement(catalog_path, data_path)
        ))
        .map_err(|source| EngineError::StorageDuckDb {
            operation: "attaching DuckLake catalog",
            name: storage_name,
            source,
        })?;
    Ok(())
}

pub(crate) fn setup_duckdb_catalog_adapter(
    connection: &duckdb::Connection,
    catalog_path: &Path,
) -> Result<(), EngineError> {
    CatalogAdapter::duckdb().setup_catalog_application_tables(connection, catalog_path)
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
        "CREATE TABLE IF NOT EXISTS pulseon_projects (
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS pulseon_runs (
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
             metric_key_encoded VARCHAR NOT NULL,
             step BIGINT NOT NULL,
             timestamp TIMESTAMPTZ NOT NULL,
             value_f64 DOUBLE NOT NULL,
             ingested_at TIMESTAMPTZ NOT NULL
         );
         ALTER TABLE dl.metric_points SET PARTITIONED BY (run_id, metric_key_encoded);
         CREATE TABLE IF NOT EXISTS pulseon_metric_aggregates (
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

    #[test]
    fn attach_ducklake_sanitizes_storage_error_paths() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-bootstrap-{}", uuid::Uuid::new_v4()));
        let catalog_path = root_path.join("private").join("catalog.ducklake");
        let data_path = root_path.join("secret-data");
        std::fs::create_dir_all(&catalog_path)?;
        std::fs::create_dir_all(&data_path)?;
        let connection = open_duckdb_connection()?;

        let error = attach_ducklake(&connection, &catalog_path, &data_path).unwrap_err();
        let message = error.to_string();

        assert!(
            matches!(error, EngineError::StorageDuckDb { .. }),
            "expected sanitized storage error, got {error:?}",
        );
        assert!(
            message.contains("attaching DuckLake catalog"),
            "expected operation in error message, got {message}",
        );
        assert!(
            message.contains("catalog.ducklake") && message.contains("secret-data"),
            "expected storage basenames in error message, got {message}",
        );
        assert!(
            !message.contains(root_path.to_string_lossy().as_ref()),
            "expected sanitized message without full path, got {message}",
        );
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn catalog_adapter_uses_duckdb_default_catalog_filename() {
        let adapter = CatalogAdapter::duckdb();

        assert_eq!(adapter.default_catalog_filename, "catalog.ducklake");
    }

    #[test]
    fn catalog_adapter_builds_ducklake_attach_statement() {
        let adapter = CatalogAdapter::duckdb();
        let statement =
            adapter.attach_ducklake_statement(Path::new("catalog.ducklake"), Path::new("data"));

        assert!(statement.contains("ATTACH 'catalog.ducklake' AS dl"));
        assert!(statement.contains("TYPE ducklake"));
        assert!(statement.contains("DATA_PATH 'data'"));
        assert!(statement.contains("METADATA_CATALOG 'pulseon_catalog'"));
    }

    #[test]
    fn create_v1_tables_partitions_metric_points_by_run_and_encoded_key()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-bootstrap-{}", uuid::Uuid::new_v4()));
        let connection = open_native_connection(&root_path)?;

        let partition_columns: Vec<String> = connection
            .prepare(
                "SELECT columns.column_name
                 FROM pulseon_catalog.ducklake_partition_column AS partitions
                 JOIN pulseon_catalog.ducklake_column AS columns
                   ON columns.column_id = partitions.column_id
                 JOIN pulseon_catalog.ducklake_table AS tables
                   ON tables.table_id = columns.table_id
                 WHERE tables.table_name = 'metric_points'
                 ORDER BY partitions.partition_key_index",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        assert_eq!(partition_columns, ["run_id", "metric_key_encoded"]);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }
}
