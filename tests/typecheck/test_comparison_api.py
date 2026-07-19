"""Pyright coverage for typed comparison read APIs."""

from __future__ import annotations

from typing import Literal, assert_type

import pulseon


def accepts_typed_comparison_reads(client: pulseon.Client) -> None:
    aligned = client.query_aligned_metric(
        "run-1",
        "loss",
        axis="elapsed_time",
        start=0,
        end=10_000,
        pixel_width=800,
        points_per_pixel=2,
    )
    comparison = client.compare_runs(
        "candidate",
        "reference",
        metric_key="loss",
        direction="minimize",
    )
    ranking = client.rank_runs(
        ["run-1", "run-2"],
        metric_key="accuracy",
        direction="maximize",
    )

    assert_type(aligned, pulseon.AlignedMetricResult)
    assert_type(aligned.points, list[pulseon.AlignedMetricPoint])
    assert_type(aligned.points[0].axis_value, int)
    assert_type(
        aligned.completeness,
        Literal["complete", "partial", "unavailable", "invalid"],
    )
    assert_type(aligned.reasons, list[str])
    assert_type(comparison, pulseon.ComparisonResult)
    assert_type(comparison.objective, pulseon.ObjectiveMetric)
    assert_type(comparison.candidate, pulseon.ObjectiveEvidence)
    assert_type(comparison.raw_delta, float | None)
    assert_type(
        comparison.outcome,
        Literal["improved", "regressed", "equal"] | None,
    )
    assert_type(ranking, pulseon.RankingResult)
    assert_type(ranking.entries, list[pulseon.RankingEntry])
    assert_type(ranking.entries[0].evidence, pulseon.ObjectiveEvidence)
    assert_type(ranking.entries[0].rank, int | None)


def rejects_invalid_literals_and_read_only_writes(client: pulseon.Client) -> None:
    client.query_aligned_metric(
        "run-1",
        "loss",
        axis="epoch",  # type: ignore[reportArgumentType]
        start=0,
        end=1,
    )
    result = client.compare_runs(
        "candidate",
        "reference",
        metric_key="loss",
        direction="lower",  # type: ignore[reportArgumentType]
    )
    result.preference = "candidate"  # type: ignore[reportAttributeAccessIssue]
