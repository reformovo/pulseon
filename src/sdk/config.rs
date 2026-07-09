use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml::Table;

use crate::engine::bootstrap::{CatalogBackend, S3ConnectionConfig, is_s3_data_path};

const MAX_METRIC_QUEUE_CAPACITY: i64 = 1_048_576;

pub(crate) struct ResolvedInitConfig {
    pub(crate) catalog_backend: CatalogBackend,
    pub(crate) catalog_path: Option<PathBuf>,
    pub(crate) data_path: Option<PathBuf>,
    pub(crate) metric_queue_capacity: usize,
    pub(crate) s3_connection: Option<S3ConnectionConfig>,
}

#[derive(Default)]
pub(crate) struct S3ConnectionOverrides {
    pub(crate) endpoint: Option<String>,
    pub(crate) access_key_id: Option<String>,
    pub(crate) secret_access_key: Option<String>,
    pub(crate) session_token: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) path_style: Option<bool>,
    pub(crate) use_ssl: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InitConfigError {
    #[error("failed to read config.toml: {source}")]
    ReadConfig {
        #[source]
        source: io::Error,
    },

    #[error("invalid config.toml: {source}")]
    ParseConfig {
        #[source]
        source: toml::de::Error,
    },

    #[error("{0}")]
    Invalid(String),
}

pub(crate) fn resolve_init_config(
    root_path: &Path,
    data_path: Option<PathBuf>,
    catalog_backend: &str,
    catalog_path: Option<PathBuf>,
    metric_queue_capacity: i64,
    s3_overrides: S3ConnectionOverrides,
) -> Result<ResolvedInitConfig, InitConfigError> {
    let config = load_project_config(root_path)?;
    let data_path = resolve_data_path(data_path, config.as_ref())?;
    validate_path_configuration(data_path.as_deref(), catalog_path.as_deref())?;

    let catalog_backend = CatalogBackend::from_name(catalog_backend)
        .ok_or_else(|| invalid(format!("unsupported catalog_backend: {catalog_backend}")))?;
    let metric_queue_capacity = validate_metric_queue_capacity(metric_queue_capacity)?;
    let s3_connection = if data_path.as_deref().is_some_and(is_s3_data_path) {
        Some(resolve_s3_connection(config.as_ref(), s3_overrides)?)
    } else {
        None
    };

    Ok(ResolvedInitConfig {
        catalog_backend,
        catalog_path,
        data_path,
        metric_queue_capacity,
        s3_connection,
    })
}

fn load_project_config(root_path: &Path) -> Result<Option<Table>, InitConfigError> {
    let config_path = root_path.join(".pulseon").join("config.toml");
    let content = match fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(InitConfigError::ReadConfig { source }),
    };
    content
        .parse::<Table>()
        .map(Some)
        .map_err(|source| InitConfigError::ParseConfig { source })
}

fn resolve_data_path(
    explicit: Option<PathBuf>,
    config: Option<&Table>,
) -> Result<Option<PathBuf>, InitConfigError> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    config
        .map(|config| optional_string(config, "data_path", "config.toml data_path"))
        .transpose()
        .map(|value| value.flatten().map(PathBuf::from))
}

fn validate_path_configuration(
    data_path: Option<&Path>,
    catalog_path: Option<&Path>,
) -> Result<(), InitConfigError> {
    if data_path.is_some_and(is_unsupported_data_uri_path) {
        return Err(invalid(
            "data_path must be a local filesystem path or s3:// URI",
        ));
    }
    if catalog_path.is_some_and(is_uri_path) {
        return Err(invalid("catalog_path must be a local filesystem path"));
    }
    Ok(())
}

fn validate_metric_queue_capacity(value: i64) -> Result<usize, InitConfigError> {
    if !(1..=MAX_METRIC_QUEUE_CAPACITY).contains(&value) {
        return Err(invalid(
            "metric_queue_capacity must be between 1 and 1048576",
        ));
    }
    usize::try_from(value)
        .map_err(|_| invalid("metric_queue_capacity must be between 1 and 1048576"))
}

fn resolve_s3_connection(
    config: Option<&Table>,
    explicit: S3ConnectionOverrides,
) -> Result<S3ConnectionConfig, InitConfigError> {
    let s3 = s3_table(config)?;
    Ok(S3ConnectionConfig::new(
        required_s3_string(
            optional_s3_string(explicit.endpoint, s3, "endpoint", "s3_endpoint")?,
            "s3_endpoint",
        )?,
        required_s3_string(
            optional_s3_string(
                explicit.access_key_id,
                s3,
                "access_key_id",
                "s3_access_key_id",
            )?,
            "s3_access_key_id",
        )?,
        required_s3_string(
            optional_s3_string(
                explicit.secret_access_key,
                s3,
                "secret_access_key",
                "s3_secret_access_key",
            )?,
            "s3_secret_access_key",
        )?,
        optional_s3_string(
            explicit.session_token,
            s3,
            "session_token",
            "s3_session_token",
        )?,
        optional_s3_string(explicit.region, s3, "region", "s3_region")?,
        optional_bool(
            explicit.path_style,
            s3,
            "path_style",
            "config.toml s3.path_style",
        )?,
        optional_bool(explicit.use_ssl, s3, "use_ssl", "config.toml s3.use_ssl")?,
    ))
}

fn s3_table(config: Option<&Table>) -> Result<Option<&Table>, InitConfigError> {
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(value) = config.get("s3") else {
        return Ok(None);
    };
    value
        .as_table()
        .ok_or_else(|| invalid("config.toml s3 must be a table"))
        .map(Some)
}

fn optional_s3_string(
    explicit: Option<String>,
    config: Option<&Table>,
    key: &str,
    explicit_name: &str,
) -> Result<Option<String>, InitConfigError> {
    if explicit.is_some() {
        return validate_s3_string(explicit, explicit_name);
    }
    let Some(value) = config
        .map(|config| optional_string(config, key, &format!("config.toml s3.{key}")))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    validate_s3_string(Some(value), explicit_name)
}

fn optional_string(
    config: &Table,
    key: &str,
    label: &str,
) -> Result<Option<String>, InitConfigError> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(format!("{label} must be a string")))
        .map(Some)
}

fn optional_bool(
    explicit: Option<bool>,
    config: Option<&Table>,
    key: &str,
    label: &str,
) -> Result<Option<bool>, InitConfigError> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("{label} must be a boolean")))
        .map(Some)
}

fn required_s3_string(value: Option<String>, name: &str) -> Result<String, InitConfigError> {
    value.ok_or_else(|| invalid(format!("{name} is required when data_path is s3://")))
}

fn validate_s3_string(
    value: Option<String>,
    name: &str,
) -> Result<Option<String>, InitConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Err(invalid(format!("{name} must not be empty")));
    }
    Ok(Some(value))
}

fn is_uri_path(path: &Path) -> bool {
    path.to_string_lossy().contains("://")
}

fn is_unsupported_data_uri_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.contains("://") && !path.starts_with("s3://")
}

fn invalid(message: impl Into<String>) -> InitConfigError {
    InitConfigError::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(raw: &str) -> Table {
        raw.parse::<Table>().expect("test config should parse")
    }

    #[test]
    fn s3_config_merges_explicit_keywords_over_file_values() {
        let config = parse_config(
            r#"
            data_path = "s3://bucket/pulseon"

            [s3]
            endpoint = "from-config:9000"
            access_key_id = "from-config"
            secret_access_key = "from-config-secret"
            path_style = false
            use_ssl = true
            "#,
        );

        let resolved = resolve_s3_connection(
            Some(&config),
            S3ConnectionOverrides {
                endpoint: Some("override:9000".to_owned()),
                path_style: Some(true),
                ..S3ConnectionOverrides::default()
            },
        )
        .expect("s3 config should resolve");

        assert_eq!(resolved.endpoint, "override:9000");
        assert_eq!(resolved.access_key_id, "from-config");
        assert_eq!(resolved.secret_access_key, "from-config-secret");
        assert_eq!(resolved.path_style, Some(true));
        assert_eq!(resolved.use_ssl, Some(true));
    }

    #[test]
    fn s3_config_skips_explicit_invalid_file_values() {
        let config = parse_config(
            r#"
            [s3]
            endpoint = 123
            access_key_id = "from-config"
            secret_access_key = "from-config-secret"
            path_style = "yes"
            use_ssl = true
            "#,
        );

        let resolved = resolve_s3_connection(
            Some(&config),
            S3ConnectionOverrides {
                endpoint: Some("override:9000".to_owned()),
                path_style: Some(true),
                ..S3ConnectionOverrides::default()
            },
        )
        .expect("explicit fields should suppress invalid file values");

        assert_eq!(resolved.endpoint, "override:9000");
        assert_eq!(resolved.access_key_id, "from-config");
        assert_eq!(resolved.secret_access_key, "from-config-secret");
        assert_eq!(resolved.path_style, Some(true));
        assert_eq!(resolved.use_ssl, Some(true));
    }

    #[test]
    fn init_config_resolves_s3_data_path_for_supported_catalog_backends() {
        let root_path =
            std::env::temp_dir().join(format!("pulseon-config-{}", uuid::Uuid::new_v4()));

        for catalog_backend in ["duckdb", "sqlite"] {
            let resolved = resolve_init_config(
                &root_path,
                Some(PathBuf::from("s3://bucket/pulseon")),
                catalog_backend,
                None,
                1024,
                S3ConnectionOverrides {
                    endpoint: Some("127.0.0.1:9000".to_owned()),
                    access_key_id: Some("pulseon-key".to_owned()),
                    secret_access_key: Some("pulseon-secret".to_owned()),
                    path_style: Some(true),
                    use_ssl: Some(false),
                    ..S3ConnectionOverrides::default()
                },
            )
            .expect("s3 init config should resolve for supported catalog backend");

            assert_eq!(
                resolved.data_path,
                Some(PathBuf::from("s3://bucket/pulseon"))
            );
            assert!(resolved.s3_connection.is_some());
        }
    }
}
