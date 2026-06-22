"""Verify that the pulseon package can be imported."""


def test_import_pulseon() -> None:
    import pulseon

    assert hasattr(pulseon, "__all__")
