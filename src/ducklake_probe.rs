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
    fn ducklake_round_trips_minimal_v1_data() -> Result<(), Box<dyn Error>> {
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
        seed_minimal_v1_data(&connection)?;

        let metric_total: f64 = connection.query_row(
            "SELECT sum(value_f64)
             FROM dl.metric_points
             WHERE run_id = 'run-1'
               AND metric_key = 'train/loss'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(metric_total, 0.375);
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

    fn seed_minimal_v1_data(connection: &duckdb::Connection) -> duckdb::Result<()> {
        connection.execute_batch(
            "INSERT INTO dl.projects VALUES
                 ('project-1', 'local training', now());
             INSERT INTO dl.runs VALUES
                 ('run-1', 'project-1', 'baseline', 'running', now(), now(), NULL);
             INSERT INTO dl.metric_points VALUES
                 ('run-1', 'train/loss', 0, now(), 0.25, now()),
                 ('run-1', 'train/loss', 1, now(), 0.125, now());
             INSERT INTO dl.metric_aggregates VALUES
                 ('run-1', 'train/loss', 2, 1, 0.125, 0.125, 0.25);",
        )
    }
}
