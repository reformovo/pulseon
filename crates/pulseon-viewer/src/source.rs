use std::path::Path;

use pulseon_model::types::Project;
use pulseon_storage::bootstrap::{
    NativeStorageConfig, is_s3_data_path, open_existing_native_connection_with_config,
};
use pulseon_storage::config::{InitConfigError, resolve_storage_config};
use pulseon_storage::{ProjectConnection, StorageError};

/// Failures while opening an existing viewer source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("S3 data paths are unsupported by pulseon-viewer")]
    UnsupportedS3,
    #[error(transparent)]
    Config(#[from] InitConfigError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Read session for one existing local native PulseOn store.
pub struct ReadSession {
    connection: ProjectConnection,
}

impl ReadSession {
    /// Opens an existing store through the shared project configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::UnsupportedS3`] before credential resolution for
    /// an S3 data path. Other configuration and storage failures are preserved.
    pub fn open_existing(root_path: &Path) -> Result<Self, SourceError> {
        let resolved = resolve_storage_config(root_path, None, None, None)?;
        if resolved.data_path.as_deref().is_some_and(is_s3_data_path) {
            return Err(SourceError::UnsupportedS3);
        }
        let config = NativeStorageConfig::with_backend_and_s3_config(
            resolved.catalog_backend,
            root_path,
            resolved.catalog_path,
            resolved.data_path,
            None,
        );
        let connection = open_existing_native_connection_with_config(config)?;
        Ok(Self {
            connection: ProjectConnection::new(connection),
        })
    }

    /// Lists Projects stored in the opened catalog.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] when the catalog query fails.
    pub fn list_projects(&self) -> Result<Vec<Project>, SourceError> {
        Ok(self.connection.list_projects()?)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn s3_is_rejected_before_missing_credentials_are_resolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let config_dir = root.path().join(".pulseon");
        fs::create_dir(&config_dir)?;
        fs::write(
            config_dir.join("config.toml"),
            "data_path = \"s3://bucket/data\"\n",
        )?;

        let error = ReadSession::open_existing(root.path()).err();

        assert!(matches!(error, Some(SourceError::UnsupportedS3)));
        Ok(())
    }
}
