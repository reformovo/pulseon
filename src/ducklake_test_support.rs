use std::path::PathBuf;

pub struct TestDataset {
    root: PathBuf,
    catalog_path: PathBuf,
    data_path: PathBuf,
}

impl TestDataset {
    pub fn new() -> std::io::Result<Self> {
        let root = std::env::temp_dir().join(format!("pulseon-ducklake-{}", uuid::Uuid::new_v4()));
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

pub fn attach_ducklake(
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

pub fn create_minimal_v1_tables(connection: &duckdb::Connection) -> duckdb::Result<()> {
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

pub fn seed_minimal_v1_data(connection: &duckdb::Connection) -> duckdb::Result<()> {
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
