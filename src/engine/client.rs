use std::path::{Path, PathBuf};

use crate::engine::EngineError;

pub struct NativeClient {
    _root_path: PathBuf,
    _connection: duckdb::Connection,
}

impl NativeClient {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let root_path = path.as_ref().to_path_buf();
        let catalog_path = root_path.join("catalog.ducklake");
        let data_path = root_path.join("data");
        std::fs::create_dir_all(&data_path)?;

        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &catalog_path, &data_path)?;
        create_v1_tables(&connection)?;

        Ok(Self {
            _root_path: root_path,
            _connection: connection,
        })
    }
}

fn attach_ducklake(
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

fn create_v1_tables(connection: &duckdb::Connection) -> Result<(), EngineError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS dl.projects (
             project_id VARCHAR NOT NULL,
             name VARCHAR NOT NULL,
             created_at TIMESTAMPTZ NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dl.runs (
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
         CREATE TABLE IF NOT EXISTS dl.metric_aggregates (
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_initializes_ducklake_dataset() -> Result<(), Box<dyn std::error::Error>> {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-client-{}", uuid::Uuid::new_v4()));

        let _client = NativeClient::open(&root_path)?;

        assert!(root_path.join("data").is_dir());
        std::fs::remove_dir_all(root_path)?;
        Ok(())
    }

    #[test]
    fn sql_string_literal_escapes_single_quotes() {
        assert_eq!(sql_string_literal("canary's/data"), "'canary''s/data'");
    }
}
