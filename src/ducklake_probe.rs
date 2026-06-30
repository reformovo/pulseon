#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::ducklake_test_support::{
        TestDataset, attach_ducklake, create_minimal_v1_tables, seed_minimal_v1_data,
    };
    use crate::model::metric::{MetricKey, Step};
    use crate::model::run::RunId;
    use crate::model::types::ProjectId;

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

    #[test]
    fn log_metric_stores_ingested_at_for_metric_points() -> Result<(), Box<dyn Error>> {
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
        let point = store.log_metric(&run.run_id, &metric_key, 0.25)?;

        // Then
        assert!(point.ingested_at >= point.timestamp);
        let stored_ingest_is_after_timestamp: bool = connection.query_row(
            "SELECT ingested_at >= timestamp
             FROM dl.metric_points
             WHERE run_id = 'run-1'
               AND metric_key = 'train/loss'
               AND step = 0",
            [],
            |row| row.get(0),
        )?;
        assert!(stored_ingest_is_after_timestamp);
        Ok(())
    }

    #[test]
    fn query_metric_uses_last_write_wins_for_duplicate_steps() -> Result<(), Box<dyn Error>> {
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
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.25)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.125)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.0625)?;

        // When
        let effective = store.query_metric_effective(&run.run_id, &metric_key)?;

        // Then
        let values: Vec<(i64, f64)> = effective
            .iter()
            .map(|point| (point.step.value(), point.value_f64))
            .collect();
        assert_eq!(values, vec![(0, 0.125), (1, 0.0625)]);
        Ok(())
    }

    #[test]
    fn query_metric_filters_effective_series_by_step_range() -> Result<(), Box<dyn Error>> {
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
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.5)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.25)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.125)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(2), 0.0625)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(3), 0.03125)?;

        // When
        let points = store.query_metric(
            &run.run_id,
            &metric_key,
            Some(Step::new(1)),
            Some(Step::new(2)),
            None,
        )?;

        // Then
        let values: Vec<(i64, f64)> = points
            .iter()
            .map(|point| (point.step.value(), point.value_f64))
            .collect();
        assert_eq!(values, vec![(1, 0.125), (2, 0.0625)]);
        Ok(())
    }

    #[test]
    fn query_metric_returns_short_series_unchanged_under_max_points() -> Result<(), Box<dyn Error>>
    {
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
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.25)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.125)?;

        // When
        let points = store.query_metric(&run.run_id, &metric_key, None, None, Some(2))?;

        // Then
        let values: Vec<(i64, f64)> = points
            .iter()
            .map(|point| (point.step.value(), point.value_f64))
            .collect();
        assert_eq!(values, vec![(0, 0.25), (1, 0.125)]);
        Ok(())
    }

    #[test]
    fn metric_aggregate_tracks_effective_series_values() -> Result<(), Box<dyn Error>> {
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
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.25)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.5)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.125)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(2), 0.375)?;

        // When
        let aggregate = store.metric_aggregate(&run.run_id, &metric_key)?;

        // Then
        assert_eq!(aggregate.effective_count, 3);
        assert_eq!(aggregate.last_step, Step::new(2));
        assert_eq!(aggregate.last_value_f64, 0.375);
        assert_eq!(aggregate.min_value_f64, 0.125);
        assert_eq!(aggregate.max_value_f64, 0.375);
        Ok(())
    }

    #[test]
    fn repair_metric_aggregate_refreshes_stale_old_step_overwrite() -> Result<(), Box<dyn Error>> {
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
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(0), 0.25)?;
        store.log_metric_at_step(&run.run_id, &metric_key, Step::new(1), 0.5)?;
        connection.execute(
            "INSERT INTO dl.metric_points VALUES
                 ('run-1', 'train/loss', 0, now(), 0.125, now())",
            [],
        )?;
        let stale = store.metric_aggregate(&run.run_id, &metric_key)?;

        // When
        store.repair_metric_aggregate(&run.run_id, &metric_key)?;
        let repaired = store.metric_aggregate(&run.run_id, &metric_key)?;

        // Then
        assert_eq!(stale.min_value_f64, 0.25);
        assert_eq!(repaired.effective_count, 2);
        assert_eq!(repaired.min_value_f64, 0.125);
        assert_eq!(repaired.max_value_f64, 0.5);
        Ok(())
    }
}
