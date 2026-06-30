#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    struct TestDataset {
        root: PathBuf,
        catalog_path: PathBuf,
        data_path: PathBuf,
    }

    impl TestDataset {
        fn new() -> std::io::Result<Self> {
            let root =
                std::env::temp_dir().join(format!("pulseon-ducklake-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root)?;
            Ok(Self {
                catalog_path: root.join("catalog.ducklake"),
                data_path: root.join("data"),
                root,
            })
        }
    }

    impl Drop for TestDataset {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn ducklake_creates_minimal_v1_tables() -> Result<(), Box<dyn Error>> {
        // Given
        let dataset = TestDataset::new()?;
        let connection = duckdb::Connection::open_in_memory()?;

        // When
        attach_ducklake(&connection, &dataset)?;
        create_minimal_v1_tables(&connection)?;

        // Then
        let table_count: i64 = connection.query_row(
            "SELECT count(*)
             FROM information_schema.tables
             WHERE table_catalog = 'dl'
               AND table_schema = 'main'
               AND table_name IN ('projects', 'runs', 'metric_points', 'metric_aggregates')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(table_count, 4);
        Ok(())
    }

    fn attach_ducklake(
        connection: &duckdb::Connection,
        dataset: &TestDataset,
    ) -> duckdb::Result<()> {
        let catalog_path = dataset.catalog_path.display();
        let data_path = dataset.data_path.display();
        connection.execute_batch(&format!(
            "INSTALL ducklake;
             LOAD ducklake;
             ATTACH '{catalog_path}' AS dl (TYPE ducklake, DATA_PATH '{data_path}');"
        ))
    }

    fn create_minimal_v1_tables(connection: &duckdb::Connection) -> duckdb::Result<()> {
        connection.execute_batch(
            "CREATE TABLE dl.projects (
                 project_id VARCHAR NOT NULL,
                 name VARCHAR NOT NULL,
                 created_at TIMESTAMPTZ NOT NULL
             );
             CREATE TABLE dl.runs (
                 run_id VARCHAR NOT NULL,
                 project_id VARCHAR NOT NULL,
                 name VARCHAR NOT NULL,
                 status VARCHAR NOT NULL,
                 created_at TIMESTAMPTZ NOT NULL,
                 started_at TIMESTAMPTZ NOT NULL,
                 finished_at TIMESTAMPTZ
             );
             CREATE TABLE dl.metric_points (
                 run_id VARCHAR NOT NULL,
                 metric_key VARCHAR NOT NULL,
                 step BIGINT NOT NULL,
                 timestamp TIMESTAMPTZ NOT NULL,
                 value_f64 DOUBLE NOT NULL,
                 ingested_at TIMESTAMPTZ NOT NULL
             );
             CREATE TABLE dl.metric_aggregates (
                 run_id VARCHAR NOT NULL,
                 metric_key VARCHAR NOT NULL,
                 effective_count UBIGINT NOT NULL,
                 last_step BIGINT NOT NULL,
                 last_value_f64 DOUBLE NOT NULL,
                 min_value_f64 DOUBLE NOT NULL,
                 max_value_f64 DOUBLE NOT NULL
             );",
        )
    }
}
