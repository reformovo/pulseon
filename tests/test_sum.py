import pulseon


def test_sum_as_string_formats_sum() -> None:
    result: str = pulseon.sum_as_string(5, 20)

    assert result == "25"
