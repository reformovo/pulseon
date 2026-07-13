"""Verify import smoke behavior."""

from __future__ import annotations


def test_import_pulseon() -> None:
    import pulseon

    assert hasattr(pulseon, "__all__")
    assert pulseon.ArrowTable.__name__ == "ArrowTable"
    assert "ArrowTable" in pulseon.__all__
