"""Opt-in MinIO acceptance coverage for S3-backed data paths."""

from __future__ import annotations

import dataclasses
import os
import pathlib
import urllib.parse
import uuid

import pytest

_CATALOG_BACKENDS = ("duckdb", "sqlite")
_REQUIRED_ENV = (
    "PULSEON_MINIO_ENDPOINT",
    "PULSEON_MINIO_BUCKET",
    "PULSEON_MINIO_ACCESS_KEY_ID",
    "PULSEON_MINIO_SECRET_ACCESS_KEY",
)


@dataclasses.dataclass(frozen=True)
class MinioConfig:
    endpoint: str
    bucket: str
    access_key_id: str
    secret_access_key: str
    session_token: str | None
    region: str | None
    use_ssl: bool

    def data_path(self, prefix: str) -> str:
        return f"s3://{self.bucket}/{prefix.strip('/')}"


@pytest.mark.parametrize("catalog_backend", _CATALOG_BACKENDS)
def test_minio_s3_data_path_initializes_catalog_backend(
    tmp_path: pathlib.Path,
    catalog_backend: str,
) -> None:
    import pulseon

    config = _require_minio_config()
    root_path = tmp_path / catalog_backend / "pulseon"
    prefix = f"pulseon-acceptance/{uuid.uuid4().hex}/{catalog_backend}"

    client = pulseon.init(
        root_path,
        data_path=config.data_path(prefix),
        catalog_backend=catalog_backend,
        s3_endpoint=config.endpoint,
        s3_access_key_id=config.access_key_id,
        s3_secret_access_key=config.secret_access_key,
        s3_session_token=config.session_token,
        s3_region=config.region,
        s3_path_style=True,
        s3_use_ssl=config.use_ssl,
    )
    project = client.create_project("minio acceptance", project_id="project-1")
    client.shutdown()

    assert project.project_id == "project-1"


def _require_minio_config() -> MinioConfig:
    missing = [name for name in _REQUIRED_ENV if not os.environ.get(name)]
    if missing:
        pytest.skip(
            "set MinIO acceptance environment variables: " + ", ".join(missing)
        )

    raw_endpoint = os.environ["PULSEON_MINIO_ENDPOINT"]
    explicit_use_ssl = os.environ.get("PULSEON_MINIO_USE_SSL")
    endpoint, endpoint_use_ssl = _normalize_endpoint(raw_endpoint)
    use_ssl = (
        _parse_bool("PULSEON_MINIO_USE_SSL", explicit_use_ssl)
        if explicit_use_ssl is not None
        else endpoint_use_ssl
    )

    return MinioConfig(
        endpoint=endpoint,
        bucket=os.environ["PULSEON_MINIO_BUCKET"],
        access_key_id=os.environ["PULSEON_MINIO_ACCESS_KEY_ID"],
        secret_access_key=os.environ["PULSEON_MINIO_SECRET_ACCESS_KEY"],
        session_token=os.environ.get("PULSEON_MINIO_SESSION_TOKEN"),
        region=os.environ.get("PULSEON_MINIO_REGION"),
        use_ssl=use_ssl,
    )


def _normalize_endpoint(raw_endpoint: str) -> tuple[str, bool]:
    parsed = urllib.parse.urlparse(raw_endpoint)
    if parsed.scheme:
        if parsed.scheme not in {"http", "https"}:
            raise AssertionError(
                "PULSEON_MINIO_ENDPOINT must use http:// or https:// when a "
                "scheme is provided"
            )
        if not parsed.netloc or parsed.path not in {"", "/"}:
            raise AssertionError(
                "PULSEON_MINIO_ENDPOINT must be host[:port] or scheme://host[:port]"
            )
        return parsed.netloc, parsed.scheme == "https"
    return raw_endpoint, False


def _parse_bool(name: str, value: str) -> bool:
    normalized = value.strip().lower()
    if normalized in {"1", "true", "yes", "on"}:
        return True
    if normalized in {"0", "false", "no", "off"}:
        return False
    raise AssertionError(f"{name} must be a boolean")
