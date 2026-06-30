#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use crate::model::metric::MetricKey;
    use crate::model::run::RunId;
    use crate::model::types::ProjectId;

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

    #[test]
    fn create_run_persists_generated_and_user_supplied_ids() -> Result<(), Box<dyn Error>> {
        // Given
        let dataset = TestDataset::new()?;
        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &dataset)?;
        create_minimal_v1_tables(&connection)?;
        connection.execute(
            "INSERT INTO dl.projects VALUES ('project-1', 'local training', now())",
            [],
        )?;
        let store = crate::engine::write::NativeWriteStore::new(&connection);
        let project_id = ProjectId::from_string("project-1");

        // When
        let generated = store.create_run(&project_id, "generated", None)?;
        let supplied = store.create_run(
            &project_id,
            "supplied",
            Some(RunId::from_string("run-user-1")),
        )?;

        // Then
        assert_ne!(generated.run_id, supplied.run_id);
        assert_eq!(supplied.run_id.as_str(), "run-user-1");
        let run_count: i64 =
            connection.query_row("SELECT count(*) FROM dl.runs", [], |row| row.get(0))?;
        assert_eq!(run_count, 2);
        Ok(())
    }

    #[test]
    fn create_run_requires_explicit_resume_for_existing_run_id() -> Result<(), Box<dyn Error>> {
        // Given
        let dataset = TestDataset::new()?;
        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &dataset)?;
        create_minimal_v1_tables(&connection)?;
        connection.execute(
            "INSERT INTO dl.projects VALUES ('project-1', 'local training', now())",
            [],
        )?;
        let store = crate::engine::write::NativeWriteStore::new(&connection);
        let project_id = ProjectId::from_string("project-1");
        let run_id = RunId::from_string("run-user-1");
        let created = store.create_run(&project_id, "first", Some(run_id.clone()))?;

        // When
        let duplicate = store.create_run(&project_id, "second", Some(run_id.clone()));
        let resumed = store.resume_run(&run_id)?;

        // Then
        assert!(matches!(
            duplicate,
            Err(crate::engine::EngineError::RunAlreadyExists { .. })
        ));
        assert_eq!(resumed, created);
        let run_count: i64 =
            connection.query_row("SELECT count(*) FROM dl.runs", [], |row| row.get(0))?;
        assert_eq!(run_count, 1);
        Ok(())
    }

    #[test]
    fn log_metric_assigns_next_step_per_run_and_metric_key() -> Result<(), Box<dyn Error>> {
        // Given
        let dataset = TestDataset::new()?;
        let connection = duckdb::Connection::open_in_memory()?;
        attach_ducklake(&connection, &dataset)?;
        create_minimal_v1_tables(&connection)?;
        connection.execute(
            "INSERT INTO dl.projects VALUES ('project-1', 'local training', now())",
            [],
        )?;
        let store = crate::engine::write::NativeWriteStore::new(&connection);
        let project_id = ProjectId::from_string("project-1");
        let run = store.create_run(&project_id, "metrics", Some(RunId::from_string("run-1")))?;
        let metric_key = MetricKey::from_string("train/loss");

        // When
        let first = store.log_metric(&run.run_id, &metric_key, 0.25)?;
        let second = store.log_metric(&run.run_id, &metric_key, 0.125)?;

        // Then
        assert_eq!(first.step.value(), 0);
        assert_eq!(second.step.value(), 1);
        let stored: Vec<(i64, f64, bool)> = connection
            .prepare(
                "SELECT step, value_f64, ingested_at IS NOT NULL
                 FROM dl.metric_points
                 WHERE run_id = 'run-1'
                   AND metric_key = 'train/loss'
                 ORDER BY step",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        assert_eq!(stored, vec![(0, 0.25, true), (1, 0.125, true)]);
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
