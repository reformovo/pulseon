use std::path::{Path, PathBuf};

use crate::engine::EngineError;

const DUCKLAKE_ALIAS: &str = "dl";
const DUCKDB_CATALOG_ALIAS: &str = "pulseon_catalog";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CatalogBackend {
    DuckDb,
    Sqlite,
}

impl CatalogBackend {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "duckdb" => Some(Self::DuckDb),
            "sqlite" => Some(Self::Sqlite),
            _ => None,
        }
    }

    fn adapter(self) -> CatalogAdapter {
        match self {
            Self::DuckDb => CatalogAdapter::duckdb(),
            Self::Sqlite => CatalogAdapter::sqlite(),
        }
    }
}

pub(crate) struct NativeStorageConfig {
    catalog_backend: CatalogBackend,
    catalog_path: PathBuf,
    data_path: PathBuf,
    s3_connection: Option<S3ConnectionConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct S3ConnectionConfig {
    pub(crate) endpoint: String,
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) path_style: Option<bool>,
    pub(crate) use_ssl: Option<bool>,
}

impl S3ConnectionConfig {
    pub(crate) fn new(
        endpoint: String,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        region: Option<String>,
        path_style: Option<bool>,
        use_ssl: Option<bool>,
    ) -> Self {
        Self {
            endpoint,
            access_key_id,
            secret_access_key,
            session_token,
            region,
            path_style,
            use_ssl,
        }
    }
}

struct CatalogAdapter {
    default_catalog_filename: &'static str,
    ducklake_alias: &'static str,
    catalog_application_database: &'static str,
    ducklake_path_prefix: &'static str,
    attach_type_clause: Option<&'static str>,
}

impl CatalogAdapter {
    const fn duckdb() -> Self {
        Self {
            default_catalog_filename: "catalog.ducklake",
            ducklake_alias: DUCKLAKE_ALIAS,
            catalog_application_database: DUCKDB_CATALOG_ALIAS,
            ducklake_path_prefix: "",
            attach_type_clause: Some("TYPE ducklake"),
        }
    }

    const fn sqlite() -> Self {
        Self {
            default_catalog_filename: "catalog.sqlite",
            ducklake_alias: DUCKLAKE_ALIAS,
            catalog_application_database: DUCKDB_CATALOG_ALIAS,
            ducklake_path_prefix: "ducklake:sqlite:",
            attach_type_clause: None,
        }
    }

    fn attach_ducklake_statement(&self, catalog_path: &Path, data_path: &Path) -> String {
        let catalog_uri = format!(
            "{}{}",
            self.ducklake_path_prefix,
            catalog_path.to_string_lossy()
        );
        let catalog_uri = sql_string_literal(&catalog_uri);
        let data_path = sql_string_literal(data_path.to_string_lossy().as_ref());
        let attach_type_clause = self
            .attach_type_clause
            .map(|clause| format!("                 {clause},\n"))
            .unwrap_or_default();
        format!(
            "ATTACH {catalog_uri} AS {} (
{attach_type_clause}                 DATA_PATH {data_path},
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
    #[cfg(test)]
    pub(crate) fn duckdb(
        root_path: &Path,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
    ) -> Self {
        Self::with_backend_and_s3_config(
            CatalogBackend::DuckDb,
            root_path,
            catalog_path,
            data_path,
            None,
        )
    }

    pub(crate) fn with_backend_and_s3_config(
        catalog_backend: CatalogBackend,
        root_path: &Path,
        catalog_path: Option<PathBuf>,
        data_path: Option<PathBuf>,
        s3_connection: Option<S3ConnectionConfig>,
    ) -> Self {
        let pulseon_path = root_path.join(".pulseon");
        let adapter = catalog_backend.adapter();
        Self {
            catalog_backend,
            catalog_path: catalog_path
                .unwrap_or_else(|| pulseon_path.join(adapter.default_catalog_filename)),
            data_path: data_path.unwrap_or_else(|| pulseon_path.join("data")),
            s3_connection,
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
    if !is_s3_data_path(&config.data_path) {
        std::fs::create_dir_all(&config.data_path).map_err(|source| EngineError::Storage {
            operation: "creating data directory",
            name: path_basename(&config.data_path),
            source,
        })?;
    }
    let _s3_connection = config.s3_connection.as_ref();

    let connection = open_duckdb_connection()?;
    attach_ducklake_with_backend(
        &connection,
        config.catalog_backend,
        &config.catalog_path,
        &config.data_path,
    )?;
    setup_catalog_adapter(&connection, config.catalog_backend, &config.catalog_path)?;
    create_v1_tables(&connection)?;
    Ok(connection)
}

#[cfg(test)]
pub(crate) fn attach_ducklake(
    connection: &duckdb::Connection,
    catalog_path: &Path,
    data_path: &Path,
) -> Result<(), EngineError> {
    attach_ducklake_with_backend(connection, CatalogBackend::DuckDb, catalog_path, data_path)
}

pub(crate) fn attach_ducklake_with_backend(
    connection: &duckdb::Connection,
    catalog_backend: CatalogBackend,
    catalog_path: &Path,
    data_path: &Path,
) -> Result<(), EngineError> {
    let storage_name = format!(
        "{}, {}",
        path_basename(catalog_path),
        path_basename(data_path)
    );
    let adapter = catalog_backend.adapter();
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

pub(crate) fn setup_catalog_adapter(
    connection: &duckdb::Connection,
    catalog_backend: CatalogBackend,
    catalog_path: &Path,
) -> Result<(), EngineError> {
    catalog_backend
        .adapter()
        .setup_catalog_application_tables(connection, catalog_path)
}

#[cfg(test)]
pub(crate) fn setup_duckdb_catalog_adapter(
    connection: &duckdb::Connection,
    catalog_path: &Path,
) -> Result<(), EngineError> {
    setup_catalog_adapter(connection, CatalogBackend::DuckDb, catalog_path)
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
         CALL dl.set_option(
             'data_inlining_row_limit',
             8192,
             table_name => 'metric_points'
         );
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

pub(crate) fn is_s3_data_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with("s3://")
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
    fn catalog_adapter_accepts_s3_data_path() {
        let adapter = CatalogAdapter::duckdb();
        let statement = adapter.attach_ducklake_statement(
            Path::new("catalog.ducklake"),
            Path::new("s3://bucket/prefix"),
        );

        assert!(statement.contains("DATA_PATH 's3://bucket/prefix'"));
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

    #[test]
    fn create_v1_tables_sets_metric_points_inlining_row_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-bootstrap-{}", uuid::Uuid::new_v4()));
        let connection = open_native_connection(&root_path)?;

        let inlining_row_limit: i64 = connection.query_row(
            "SELECT CAST(value AS BIGINT)
             FROM pulseon_catalog.ducklake_metadata AS metadata
             JOIN pulseon_catalog.ducklake_table AS tables
               ON tables.table_id = metadata.scope_id
             WHERE tables.table_name = 'metric_points'
               AND metadata.scope = 'table'
               AND metadata.key = 'data_inlining_row_limit'",
            [],
            |row| row.get(0),
        )?;

        assert_eq!(inlining_row_limit, 8192);
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }
}
