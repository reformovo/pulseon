"""Verify that the pulseon package can be imported."""

import pathlib


def test_import_pulseon() -> None:
    import pulseon

    assert hasattr(pulseon, "__all__")


def test_init_returns_client(tmp_path: pathlib.Path) -> None:
    import pulseon

    client = pulseon.init(tmp_path / "pulseon")

    assert isinstance(client, pulseon.Client)
